use crate::{doc_loader::Document, error::ServerError};
use async_openai::{
    Client as OpenAIClient, config::OpenAIConfig, error::ApiError as OpenAIAPIErr,
    types::CreateEmbeddingRequestArgs,
};
use futures::stream::{self, StreamExt};
use ndarray::{Array1, ArrayView1};
use std::sync::Arc;
use std::sync::OnceLock;
use tiktoken_rs::cl100k_base;

// Static OnceLock for the OpenAI client
pub static OPENAI_CLIENT: OnceLock<OpenAIClient<OpenAIConfig>> = OnceLock::new();

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents supported embedding providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub enum EmbeddingProvider {
    OpenAI,
    Onnx,
    // Can be extended with other providers
}

impl fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmbeddingProvider::OpenAI => write!(f, "OpenAI"),
            EmbeddingProvider::Onnx => write!(f, "ONNX"),
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
    pub content: String,  // The extracted document content
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
#[allow(dead_code)]
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

