use rmcp::ServiceError; // Assuming ServiceError is the correct top-level error
use thiserror::Error;
use crate::doc_loader::DocLoaderError; // Need to import DocLoaderError from the sibling module

// Define a Result type alias for convenience
pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Environment variable not set: {0}")]
    MissingEnvVar(String),
    // MissingArgument removed as clap handles this now
    #[error("Configuration Error: {0}")]
    Config(String),

    #[error("MCP Service Error: {0}")]
    Mcp(#[from] ServiceError), // Use ServiceError
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Document Loading Error: {0}")]
    DocLoader(#[from] DocLoaderError),
    #[error("OpenAI Error: {0}")]
    OpenAI(#[from] async_openai::error::OpenAIError),
    #[error("JSON Error: {0}")]
    Json(#[from] serde_json::Error), // Add error for JSON deserialization
    #[error("Tiktoken Error: {0}")]
    Tiktoken(String),
    #[error("XDG Directory Error: {0}")]
    Xdg(String),
    #[error("MCP Runtime Error: {0}")]
    McpRuntime(String),
    
    // New errors for embedding cache service
    #[error("Embedding Provider Error: {0}")]
    #[allow(dead_code)]
    EmbeddingProvider(String),
    #[error("Embedding Cache Error: {0}")]
    #[allow(dead_code)]
    EmbeddingCache(String),
    #[error("Embedding Dimension Mismatch: expected {expected}, got {actual}")]
    #[allow(dead_code)]
    EmbeddingDimensionMismatch { expected: usize, actual: usize },
    #[error("Unsupported Model Error: {0}")]
    #[allow(dead_code)]
    UnsupportedModel(String),
    #[error("Bincode Error: {0}")]
    Bincode(#[from] bincode::error::EncodeError),
    #[error("Bincode Decode Error: {0}")]
    BincodeDecode(#[from] bincode::error::DecodeError),
    
    // HTTP client errors
    #[error("HTTP Request Error: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("HTTP Transport Error: {0}")]
    #[allow(dead_code)]
    HttpTransport(String),
}