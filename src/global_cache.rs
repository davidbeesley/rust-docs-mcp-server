use crate::{
    error::ServerError,
    fast_hash,
};
use std::{
    collections::HashMap,
    fs,
    io::{BufReader, BufWriter},
    path::PathBuf,
    sync::RwLock,
};
use bincode::config;

#[cfg(not(target_os = "windows"))]
use xdg::BaseDirectories;

// Global in-memory cache to avoid filesystem lookups for already loaded embeddings
lazy_static::lazy_static! {
    static ref EMBEDDING_CACHE: RwLock<HashMap<u64, Vec<f32>>> = RwLock::new(HashMap::new());
}

/// Gets the path to the global embeddings cache directory
fn get_cache_dir() -> Result<PathBuf, ServerError> {
    #[cfg(not(target_os = "windows"))]
    {
        let xdg_dirs = BaseDirectories::with_prefix("rustdocs-mcp-server")
            .map_err(|e| ServerError::Xdg(format!("Failed to get XDG directories: {}", e)))?;
        let cache_dir = xdg_dirs.get_data_home().join("embeddings-v2");
        fs::create_dir_all(&cache_dir).map_err(ServerError::Io)?;
        Ok(cache_dir)
    }

    #[cfg(target_os = "windows")]
    {
        use dirs;
        let cache_dir = dirs::cache_dir()
            .ok_or_else(|| ServerError::Config("Could not determine cache directory on Windows".to_string()))?
            .join("rustdocs-mcp-server")
            .join("embeddings-v2");
        fs::create_dir_all(&cache_dir).map_err(ServerError::Io)?;
        Ok(cache_dir)
    }
}

/// Get embedding for a document from the cache (either in-memory or disk)
pub fn get_embedding(document_content: &str) -> Option<Vec<f32>> {
    // First compute the content hash
    let content_hash = fast_hash::compute_content_hash(document_content);
    
    // Check in-memory cache first
    {
        let cache_read = EMBEDDING_CACHE.read().unwrap();
        if let Some(embedding) = cache_read.get(&content_hash) {
            return Some(embedding.clone());
        }
    }
    
    // If not in memory, try to load from disk
    match get_cache_dir() {
        Ok(cache_dir) => {
            let embedding_path = cache_dir.join(format!("{:016x}.bin", content_hash));
            if embedding_path.exists() {
                match fs::File::open(&embedding_path) {
                    Ok(file) => {
                        let reader = BufReader::new(file);
                        match bincode::decode_from_reader::<Vec<f32>, _, _>(reader, config::standard()) {
                            Ok(embedding) => {
                                // Add to in-memory cache
                                let mut cache_write = EMBEDDING_CACHE.write().unwrap();
                                cache_write.insert(content_hash, embedding.clone());
                                Some(embedding)
                            },
                            Err(_) => None,
                        }
                    },
                    Err(_) => None,
                }
            } else {
                None
            }
        },
        Err(_) => None,
    }
}

/// Store an embedding in the global cache
pub fn store_embedding(document_content: &str, embedding: &[f32]) -> Result<(), ServerError> {
    // First compute the content hash
    let content_hash = fast_hash::compute_content_hash(document_content);
    
    // Store in in-memory cache
    {
        let mut cache_write = EMBEDDING_CACHE.write().unwrap();
        cache_write.insert(content_hash, embedding.to_vec());
    }
    
    // Also store on disk
    let cache_dir = get_cache_dir()?;
    let embedding_path = cache_dir.join(format!("{:016x}.bin", content_hash));
    
    let file = fs::File::create(&embedding_path).map_err(ServerError::Io)?;
    let mut writer = BufWriter::new(file);
    
    bincode::encode_into_std_write(embedding, &mut writer, config::standard())
        .map_err(|e| ServerError::Config(format!("Failed to encode embedding: {}", e)))?;
    
    Ok(())
}

/// Batch store multiple embeddings
pub fn store_embeddings_batch(content_embedding_pairs: &[(String, Vec<f32>)]) -> Result<(), ServerError> {
    for (content, embedding) in content_embedding_pairs {
        store_embedding(content, embedding)?;
    }
    Ok(())
}