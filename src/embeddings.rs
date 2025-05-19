use crate::{doc_loader::Document, error::ServerError};
use async_openai::{
    config::OpenAIConfig, error::ApiError as OpenAIAPIErr, types::CreateEmbeddingRequestArgs,
    Client as OpenAIClient,
};
use ndarray::{Array1, ArrayView1};
use std::sync::OnceLock;
use std::sync::Arc;
use tiktoken_rs::cl100k_base;
use futures::stream::{self, StreamExt};

// Static OnceLock for the OpenAI client
pub static OPENAI_CLIENT: OnceLock<OpenAIClient<OpenAIConfig>> = OnceLock::new();

use bincode::{Encode, Decode};
use serde::{Serialize, Deserialize};
use std::fmt;

/// Represents supported embedding providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub enum EmbeddingProvider {
    OpenAI,
    ONNX,
    // Can be extended with other providers
}

impl fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbeddingProvider::OpenAI => write!(f, "OpenAI"),
            EmbeddingProvider::ONNX => write!(f, "ONNX"),
        }
    }
}

/// Represents an embedding vector with metadata
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Embedding {
    /// The actual embedding vector values
    pub values: Vec<f32>,
    /// Which provider generated this embedding
    pub provider: EmbeddingProvider,
    /// The model used to generate this embedding
    pub model: String,
    /// Dimension of the embedding vector
    pub dimensions: usize,
}

impl Embedding {
    /// Creates a new Embedding instance
    pub fn new(vector: Vec<f32>, provider: EmbeddingProvider, model: String) -> Self {
        let dimensions = vector.len();
        Self {
            values: vector,
            provider,
            model,
            dimensions,
        }
    }
    
    /// Converts the embedding to an ndarray::Array1 for numerical operations
    pub fn to_array(&self) -> Array1<f32> {
        Array1::from(self.values.clone())
    }
}

// Define a struct containing path, content, and embedding for caching
#[derive(Serialize, Deserialize, Debug, Encode, Decode)]
pub struct CachedDocumentEmbedding {
    pub path: String,
    pub content: String, // The extracted document content
    pub vector: Vec<f32>, // Keep this as 'vector' for backward compatibility with main.rs
}

/// Result type specific to embedding operations
pub type EmbeddingResult<T> = std::result::Result<T, crate::error::ServerError>;


/// Calculates the cosine similarity between two vectors.
pub fn cosine_similarity(v1: ArrayView1<f32>, v2: ArrayView1<f32>) -> f32 {
    let dot_product = v1.dot(&v2);
    let norm_v1 = v1.dot(&v1).sqrt();
    let norm_v2 = v2.dot(&v2).sqrt();

    if norm_v1 == 0.0 || norm_v2 == 0.0 {
        0.0
    } else {
        dot_product / (norm_v1 * norm_v2)
    }
}

/// Calculates the cosine similarity between two Embedding instances.
/// Returns an error if the embeddings have different dimensions.
pub fn embedding_similarity(e1: &Embedding, e2: &Embedding) -> EmbeddingResult<f32> {
    if e1.dimensions != e2.dimensions {
        return Err(ServerError::EmbeddingDimensionMismatch {
            expected: e1.dimensions,
            actual: e2.dimensions,
        });
    }
    
    let v1 = e1.to_array();
    let v2 = e2.to_array();
    
    Ok(cosine_similarity(v1.view(), v2.view()))
}

/// Generates embeddings for a list of documents using the OpenAI API.
pub async fn generate_embeddings(
    client: &OpenAIClient<OpenAIConfig>,
    documents: &[Document],
    model: &str,
) -> EmbeddingResult<(Vec<(String, Embedding)>, usize)> { // Return tuple: (embeddings, total_tokens)
    // eprintln!("Generating embeddings for {} documents...", documents.len());

    // Get the tokenizer for the model and wrap in Arc
    let bpe = Arc::new(cl100k_base().map_err(|e| ServerError::Tiktoken(e.to_string()))?);

    const CONCURRENCY_LIMIT: usize = 8; // Number of concurrent requests
    const TOKEN_LIMIT: usize = 8000; // Keep a buffer below the 8192 limit

    let results = stream::iter(documents.iter().enumerate())
        .map(|(index, doc)| {
            // Clone client, model, doc, and Arc<BPE> for the async block
            let client = client.clone();
            let model = model.to_string();
            let doc = doc.clone();
            let bpe = Arc::clone(&bpe); // Clone the Arc pointer

            async move {
                // Calculate token count for this document
                let token_count = bpe.encode_with_special_tokens(&doc.content).len();

                if token_count > TOKEN_LIMIT {
                    // eprintln!(
                    //     "    Skipping document {}: Actual tokens ({}) exceed limit ({}). Path: {}",
                    //     index + 1,
                    //     token_count,
                    //     TOKEN_LIMIT,
                    //     doc.path
                    // );
                    // Return Ok(None) to indicate skipping, with 0 tokens processed for this doc
                    return Ok::<Option<(String, Embedding, usize)>, ServerError>(None);
                }

                // Prepare input for this single document
                let inputs: Vec<String> = vec![doc.content.clone()];

                let request = CreateEmbeddingRequestArgs::default()
                    .model(&model) // Use cloned model string
                    .input(inputs)
                    .build()?; // Propagates OpenAIError

                // eprintln!(
                //     "    Sending request for document {} ({} tokens)... Path: {}",
                //     index + 1,
                //     token_count, // Use correct variable name
                //     doc.path
                // );
                let response = client.embeddings().create(request).await?; // Propagates OpenAIError
                // eprintln!("    Received response for document {}.", index + 1);

                if response.data.len() != 1 {
                    return Err(ServerError::OpenAI(
                        async_openai::error::OpenAIError::ApiError(OpenAIAPIErr {
                            message: format!(
                                "Mismatch in response length for document {}. Expected 1, got {}.",
                                index + 1, response.data.len()
                            ),
                            r#type: Some("sdk_error".to_string()),
                            param: None,
                            code: None,
                        }),
                    ));
                }

                // Process result
                let embedding_data = response.data.first().unwrap(); // Safe unwrap due to check above
                let vector = embedding_data.embedding.clone();
                
                // Create an Embedding struct
                let embedding = Embedding::new(
                    vector,
                    EmbeddingProvider::OpenAI,
                    model.clone(),
                );
                
                // Return Ok(Some(...)) for successful embedding, include token count
                Ok(Some((doc.path.clone(), embedding, token_count))) 
            }
        })
        .buffer_unordered(CONCURRENCY_LIMIT) // Run up to CONCURRENCY_LIMIT futures concurrently
        .collect::<Vec<Result<Option<(String, Embedding, usize)>, ServerError>>>()
        .await;

    // Process collected results, filtering out errors and skipped documents, summing tokens
    let mut embeddings_vec = Vec::new();
    let mut total_processed_tokens: usize = 0;
    for result in results {
        match result {
            Ok(Some((path, embedding, tokens))) => {
                embeddings_vec.push((path, embedding)); // Keep successful embeddings
                total_processed_tokens += tokens; // Add tokens for successful ones
            }
            Ok(None) => {} // Ignore skipped documents
            Err(e) => {
                // Log error but potentially continue? Or return the first error?
                // For now, let's return the first error encountered.
                eprintln!("Error during concurrent embedding generation: {}", e);
                return Err(e);
            }
        }
    }

    eprintln!(
        "Finished generating embeddings. Successfully processed {} documents ({} tokens).",
        embeddings_vec.len(), total_processed_tokens
    );
    Ok((embeddings_vec, total_processed_tokens)) // Return tuple
}