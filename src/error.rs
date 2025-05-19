use crate::doc_loader::DocLoaderError;
use rmcp::ServiceError; // Assuming ServiceError is the correct top-level error
use thiserror::Error; // Need to import DocLoaderError from the sibling module

// Define a Result type alias for convenience
pub type Result<T> = std::result::Result<T, ServerError>;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("Environment variable not set: {0}")]
    MissingEnvVar(String),

    #[error("MCP Service Error: {0}")]
    Mcp(#[from] ServiceError),
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Document Loading Error: {0}")]
    DocLoader(#[from] DocLoaderError),
    #[error("OpenAI Error: {0}")]
    OpenAI(#[from] async_openai::error::OpenAIError),
    #[error("JSON Error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP Runtime Error: {0}")]
    McpRuntime(String),

    // This variant is needed by the with_context utility function
    #[allow(dead_code)]
    #[error("Configuration Error: {0}")]
    Config(String),

    // Embedding related errors
    #[error("Embedding Dimension Mismatch: expected {expected}, got {actual}")]
    EmbeddingDimensionMismatch { expected: usize, actual: usize },
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
