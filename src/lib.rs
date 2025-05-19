// Export modules for use in examples and tests
pub mod doc_loader;
pub mod document_chunker;
pub mod embeddings;
pub mod embedding_cache_service;
pub mod error;
pub mod server;

// Re-export commonly used types for convenience
pub use doc_loader::Document;
pub use document_chunker::{DocumentChunker, Chunk};
pub use embeddings::{Embedding, EmbeddingProvider};
pub use embedding_cache_service::EmbeddingCacheService;
pub use error::{Result, ServerError};
pub use server::RustDocsServer;