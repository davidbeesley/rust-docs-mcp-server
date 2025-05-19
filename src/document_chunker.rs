use fnv::FnvHasher;
use sha2::{Digest, Sha256};
use std::hash::{Hash, Hasher};

/// Default values for the chunker
const DEFAULT_MIN_CHUNK_SIZE: usize = 1000; // ~1KB minimum
const DEFAULT_TARGET_CHUNK_SIZE: usize = 4000; // ~4KB target
const DEFAULT_MAX_CHUNK_SIZE: usize = 8000; // ~8KB maximum

/// Polynomial used for rolling hash function (prime number)
const POLYNOMIAL: u32 = 69997;

/// Bit mask for determining chunk boundaries (2^13-1)
const CHUNK_MASK: u32 = 0x1FFF;

/// Implements Content-Defined Chunking (CDC) for documents.
/// Uses a rolling hash function to find natural chunk boundaries based on content.
#[derive(Debug, Clone)]
pub struct DocumentChunker {
    min_chunk_size: usize,
    target_chunk_size: usize,
    max_chunk_size: usize,
}

/// Represents a single chunk from a document
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Unique identifier for the chunk based on its content
    pub id: String,
    /// The content of the chunk
    pub content: String,
}

impl DocumentChunker {
    /// Creates a new DocumentChunker with default parameters
    pub fn new() -> Self {
        Self {
            min_chunk_size: DEFAULT_MIN_CHUNK_SIZE,
            target_chunk_size: DEFAULT_TARGET_CHUNK_SIZE,
            max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
        }
    }

    /// Returns the minimum chunk size
    pub fn min_chunk_size(&self) -> usize {
        self.min_chunk_size
    }

    /// Returns the target chunk size
    #[allow(dead_code)]
    pub fn target_chunk_size(&self) -> usize {
        self.target_chunk_size
    }

    /// Returns the maximum chunk size
    #[allow(dead_code)]
    pub fn max_chunk_size(&self) -> usize {
        self.max_chunk_size
    }

    /// Creates a new DocumentChunker with custom parameters
    #[allow(dead_code)]
    pub fn with_params(min_size: usize, target_size: usize, max_size: usize) -> Self {
        Self {
            min_chunk_size: min_size,
            target_chunk_size: target_size,
            max_chunk_size: max_size,
        }
    }

    /// Creates a new chunk with content and ID
    fn create_chunk(&self, content: &str) -> Chunk {
        Chunk {
            id: self.generate_chunk_id(content),
            content: content.to_string(),
        }
    }

    /// Process document for chunk boundaries using rolling hash
    fn find_chunk_boundaries(&self, document: &str) -> Vec<usize> {
        let bytes = document.as_bytes();
        let mut boundaries = Vec::new();
        let mut start_idx = 0;
        let mut i = 0;
        let mut rolling_hash: u32 = 0;

        while i < bytes.len() {
            // Update rolling hash with next byte
            rolling_hash = ((rolling_hash << 1) | (bytes[i] as u32)) % POLYNOMIAL;
            i += 1;

            // Only consider boundaries after minimum chunk size
            if i - start_idx < self.min_chunk_size {
                continue;
            }

            // Forced break at maximum chunk size
            if i - start_idx >= self.max_chunk_size {
                boundaries.push(i);
                start_idx = i;
                rolling_hash = 0;
                continue;
            }

            // Check if rolling hash matches chunk boundary pattern
            // We use a bit mask to create breakpoints with a certain probability
            if (rolling_hash & CHUNK_MASK) == 0 || (i - start_idx >= self.target_chunk_size) {
                boundaries.push(i);
                start_idx = i;
                rolling_hash = 0;
            }
        }

        // Add the end of document if not already included
        if !boundaries.is_empty() && boundaries[boundaries.len() - 1] != bytes.len() {
            boundaries.push(bytes.len());
        }

        boundaries
    }

    /// Splits a document into content-defined chunks
    pub fn chunk_document(&self, document: &str) -> Vec<Chunk> {
        // Handle small documents that don't need chunking
        if document.len() <= self.min_chunk_size {
            return vec![self.create_chunk(document)];
        }

        // Find all chunk boundaries
        let boundaries = self.find_chunk_boundaries(document);

        // No boundaries found, just return the whole document
        if boundaries.is_empty() {
            return vec![self.create_chunk(document)];
        }

        // Create chunks from the boundaries
        let mut chunks = Vec::with_capacity(boundaries.len());
        let mut start_idx = 0;

        for boundary in boundaries {
            let chunk_content = &document[start_idx..boundary];
            chunks.push(self.create_chunk(chunk_content));
            start_idx = boundary;
        }

        chunks
    }

    /// Generates a stable unique identifier for a chunk based on its content
    pub fn generate_chunk_id(&self, content: &str) -> String {
        // Use SHA-256 for content-based ID
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Alternative method using FNV hasher (faster but less collision-resistant)
    fn _generate_quick_chunk_id(&self, content: &str) -> u64 {
        let mut hasher = FnvHasher::default();
        content.hash(&mut hasher);
        hasher.finish()
    }
}

impl Default for DocumentChunker {
    fn default() -> Self {
        Self::new()
    }
}
