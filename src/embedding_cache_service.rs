use std::fs;
use std::path::{Path, PathBuf};
use std::env;
use std::collections::HashMap;
use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};
use reqwest::Client;

use crate::error::Result;
use crate::embeddings::{Embedding, EmbeddingProvider};
use crate::document_chunker::{DocumentChunker, Chunk};

#[derive(Debug)]
pub struct EmbeddingCacheService {
    cache_dir: PathBuf,
    client: Client,
    openai_api_key: String,
    chunker: DocumentChunker,
}

#[derive(Serialize, Deserialize)]
struct CachedEmbedding {
    vector: Vec<f32>,  // This remains 'vector' for serialization
    document: String,
    model: String,
    provider: EmbeddingProvider,
}

impl EmbeddingCacheService {
    pub fn new(openai_api_key: String) -> Self {
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        let cache_dir = home_dir.join(".rust-doc-embedding-cache");
        fs::create_dir_all(&cache_dir).expect("Failed to create cache directory");
        
        Self {
            cache_dir,
            client: Client::new(),
            openai_api_key,
            chunker: DocumentChunker::new(),
        }
    }
    
    /// Creates a new service with custom chunker parameters
    pub fn with_chunker_params(openai_api_key: String, min_size: usize, target_size: usize, max_size: usize) -> Self {
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        let cache_dir = home_dir.join(".rust-doc-embedding-cache");
        fs::create_dir_all(&cache_dir).expect("Failed to create cache directory");
        
        Self {
            cache_dir,
            client: Client::new(),
            openai_api_key,
            chunker: DocumentChunker::with_params(min_size, target_size, max_size),
        }
    }

    /// Compute the cache path for a chunk based on its ID
    fn cache_path(&self, chunk_id: &str) -> PathBuf {
        self.cache_dir.join(chunk_id)
    }

    /// Get embedding for a document by chunking it first
    pub async fn get_embedding(&self, document: &str) -> Result<Embedding> {
        // For small documents, don't bother chunking
        if document.len() < self.chunker.min_chunk_size() {
            return self.get_embedding_for_chunk(document).await;
        }
        
        // Use chunking for larger documents
        let chunks = self.chunker.chunk_document(document);
        
        // If there's only one chunk, process it directly
        if chunks.len() == 1 {
            return self.get_embedding_for_chunk(&chunks[0].content).await;
        }
        
        // Process all chunks and combine their embeddings
        let mut chunk_embeddings = HashMap::new();
        for chunk in chunks {
            let cache_path = self.cache_path(&chunk.id);
            
            let embedding = if cache_path.exists() {
                self.read_cached_embedding(&cache_path, &chunk.content)?
            } else {
                self.generate_and_cache_embedding(&chunk.content, &cache_path).await?
            };
            
            chunk_embeddings.insert(chunk.id, embedding);
        }
        
        // Return the combined embedding (average all chunk embeddings)
        self.combine_chunk_embeddings(chunk_embeddings)
    }
    
    /// Get embedding for a single chunk of content
    pub async fn get_embedding_for_chunk(&self, chunk_content: &str) -> Result<Embedding> {
        // Generate chunk ID
        let chunk_id = self.chunker.generate_chunk_id(chunk_content);
        let cache_path = self.cache_path(&chunk_id);

        if cache_path.exists() {
            return self.read_cached_embedding(&cache_path, chunk_content);
        }

        let embedding = self.generate_and_cache_embedding(chunk_content, &cache_path).await?;
        Ok(embedding)
    }
    
    /// Combine multiple chunk embeddings into a single document embedding
    fn combine_chunk_embeddings(&self, chunk_embeddings: HashMap<String, Embedding>) -> Result<Embedding> {
        if chunk_embeddings.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData, 
                "No chunk embeddings to combine"
            ).into());
        }
        
        // Ensure all embeddings have the same dimensionality
        let first_embedding = chunk_embeddings.values().next().unwrap();
        let dim = first_embedding.dimensions;
        let model = first_embedding.model.clone();
        
        // Initialize sum vector with zeros
        let mut sum_vector = vec![0.0; dim];
        
        // Sum all vectors
        for embedding in chunk_embeddings.values() {
            if embedding.dimensions != dim {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData, 
                    "Cannot combine embeddings with different dimensions"
                ).into());
            }
            
            for (i, val) in embedding.values.iter().enumerate() {
                sum_vector[i] += val;
            }
        }
        
        // Normalize the resulting vector
        let count = chunk_embeddings.len() as f32;
        for val in &mut sum_vector {
            *val /= count;
        }
        
        // Normalize to unit length
        let magnitude: f32 = sum_vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut sum_vector {
                *val /= magnitude;
            }
        }
        
        Ok(Embedding::new(
            sum_vector,
            EmbeddingProvider::OpenAI,
            model,
        ))
    }

    fn read_cached_embedding(&self, path: &Path, original_document: &str) -> Result<Embedding> {
        let cached_data = fs::read_to_string(path)?;
        let cached: CachedEmbedding = serde_json::from_str(&cached_data)?;
        
        // Verify document matches to prevent hash collisions
        if cached.document != original_document {
            // Document changed, need to regenerate
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData, 
                "Cached document doesn't match input"
            ).into());
        }
        
        // Clone the vector to avoid moving it
        let vector_clone = cached.vector.clone();
        let dimensions = cached.vector.len();
        
        Ok(Embedding {
            values: vector_clone,
            provider: cached.provider,
            model: cached.model,
            dimensions,
        })
    }

    async fn generate_and_cache_embedding(&self, document: &str, cache_path: &Path) -> Result<Embedding> {
        // OpenAI API call
        let embedding = self.generate_openai_embedding(document).await?;
        
        // Cache the result
        let cached = CachedEmbedding {
            vector: embedding.values.clone(),
            document: document.to_string(),
            model: embedding.model.clone(),
            provider: embedding.provider,
        };
        
        let json = serde_json::to_string(&cached)?;
        fs::write(cache_path, json)?;
        
        Ok(embedding)
    }

    async fn generate_openai_embedding(&self, document: &str) -> Result<Embedding> {
        #[derive(Serialize)]
        struct EmbeddingRequest {
            input: String,
            model: String,
        }

        #[derive(Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        #[derive(Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
            model: String,
        }

        // Get the embedding model from environment or use default
        let model = env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());

        let request = EmbeddingRequest {
            input: document.to_string(),
            model: model.clone(),
        };

        let response = self.client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.openai_api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("OpenAI API error: {}", response.status())
            ).into());
        }

        let embedding_response: EmbeddingResponse = response.json().await?;
        
        // Extract the embedding values from the response
        if let Some(data) = embedding_response.data.first() {
            Ok(Embedding::new(
                data.embedding.clone(),
                EmbeddingProvider::OpenAI,
                embedding_response.model,
            ))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "No embedding data received from OpenAI"
            ).into())
        }
    }
}