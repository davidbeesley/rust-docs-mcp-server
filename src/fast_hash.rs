use std::hash::Hasher;

/// A very fast, non-cryptographic hash function (FNV-1a)
/// Used specifically for content hashing where we prioritize speed over security
pub struct FastHasher {
    state: u64,
}

impl FastHasher {
    pub fn new() -> Self {
        // FNV-1a initialization value
        Self { state: 0xcbf29ce484222325 }
    }
    
    pub fn hash_string(s: &str) -> u64 {
        let mut hasher = Self::new();
        for byte in s.bytes() {
            hasher.write_u8(byte);
        }
        hasher.finish()
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }
    
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u8(byte);
        }
    }
    
    #[inline]
    fn write_u8(&mut self, byte: u8) {
        // FNV-1a algorithm
        const FNV_PRIME: u64 = 0x100000001b3;
        self.state ^= byte as u64;
        self.state = self.state.wrapping_mul(FNV_PRIME);
    }
}

/// Normalize content before hashing to ensure consistent hash values
/// This removes whitespace variations that don't affect semantic content
pub fn normalize_content(content: &str) -> String {
    content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Compute a fast content hash for document content that can be used
/// across projects and feature configurations
pub fn compute_content_hash(content: &str) -> u64 {
    let normalized = normalize_content(content);
    FastHasher::hash_string(&normalized)
}