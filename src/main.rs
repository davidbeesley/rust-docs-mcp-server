// Declare modules
mod doc_loader;
mod embeddings;
mod error;
mod server;

// Use necessary items from modules and crates
use crate::{
    doc_loader::Document,
    embeddings::{generate_embeddings, CachedDocumentEmbedding, OPENAI_CLIENT},
    error::ServerError,
    server::RustDocsServer,
};
use async_openai::{Client as OpenAIClient, config::OpenAIConfig};
use bincode::config;
use clap::Parser;
use ndarray::Array1;
use rmcp::{transport::io::stdio, ServiceExt};
use std::{
    collections::hash_map::DefaultHasher,
    env,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::BufReader,
    path::{Path, PathBuf},
};
#[cfg(not(target_os = "windows"))]
use xdg::BaseDirectories;

// --- CLI Argument Parsing ---

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Crate names to load documentation for.
    #[arg(value_delimiter = ',')]
    crate_names: Vec<String>,

    /// Path to workspace root directory (where target/doc is located)
    #[arg(short = 'w', long, default_value = ".")]
    workspace_path: PathBuf,
    
    /// Optional features enabled when the documentation was generated.
    #[arg(short = 'F', long, value_delimiter = ',', num_args = 0..)]
    features: Option<Vec<String>>,
}

// Helper function to create a stable hash from features
fn hash_features(features: &Option<Vec<String>>) -> String {
    features
        .as_ref()
        .map(|f| {
            let mut sorted_features = f.clone();
            sorted_features.sort_unstable(); // Sort for consistent hashing
            let mut hasher = DefaultHasher::new();
            sorted_features.hash(&mut hasher);
            format!("{:x}", hasher.finish()) // Return hex representation of hash
        })
        .unwrap_or_else(|| "no_features".to_string())
}

// Cache file path helper
fn get_cache_file_path(crate_name: &str, features_hash: &str) -> Result<PathBuf, ServerError> {
    let embeddings_relative_path = PathBuf::from(crate_name)
        .join(features_hash)
        .join("embeddings.bin");

    #[cfg(not(target_os = "windows"))]
    let embeddings_file_path = {
        let xdg_dirs = BaseDirectories::with_prefix("rustdocs-mcp-server")
            .map_err(|e| ServerError::Xdg(format!("Failed to get XDG directories: {}", e)))?;
        xdg_dirs
            .place_data_file(embeddings_relative_path)
            .map_err(ServerError::Io)?
    };

    #[cfg(target_os = "windows")]
    let embeddings_file_path = {
        let cache_dir = dirs::cache_dir().ok_or_else(|| {
            ServerError::Config("Could not determine cache directory on Windows".to_string())
        })?;
        let app_cache_dir = cache_dir.join("rustdocs-mcp-server");
        fs::create_dir_all(&app_cache_dir).map_err(ServerError::Io)?;
        app_cache_dir.join(embeddings_relative_path)
    };

    Ok(embeddings_file_path)
}

// Helper to save embeddings to cache file
fn save_embeddings_to_cache(
    cache_file_path: &Path,
    documents: &[Document],
    embeddings: &[(String, Array1<f32>)]
) -> Result<(), ServerError> {
    // Create a map for easy lookup of embeddings by path
    let embedding_map: std::collections::HashMap<&String, &Array1<f32>> =
        embeddings.iter().map(|(path, embedding)| (path, embedding)).collect();
    
    // Create cache entries with content hashes
    let mut combined_cache_data: Vec<CachedDocumentEmbedding> = Vec::new();
    
    for doc in documents {
        if let Some(embedding_array) = embedding_map.get(&doc.path) {
            // Compute content hash for version-independent caching
            let content_hash = embeddings::compute_content_hash(&doc.content);
            
            combined_cache_data.push(CachedDocumentEmbedding {
                path: doc.path.clone(),
                content: doc.content.clone(),
                content_hash,
                vector: embedding_array.to_vec(),
            });
        } else {
            eprintln!(
                "Warning: Embedding not found for document path: {}. Skipping from cache.",
                doc.path
            );
        }
    }
    
    // Ensure parent directory exists
    if let Some(parent_dir) = cache_file_path.parent() {
        if !parent_dir.exists() {
            if let Err(e) = fs::create_dir_all(parent_dir) {
                eprintln!(
                    "Warning: Failed to create cache directory {}: {}",
                    parent_dir.display(), e
                );
                return Err(ServerError::Io(e));
            }
        }
    }
    
    // Write cache file
    match bincode::encode_to_vec(&combined_cache_data, config::standard()) {
        Ok(encoded_bytes) => {
            if let Err(e) = fs::write(cache_file_path, encoded_bytes) {
                eprintln!("Warning: Failed to write cache file: {}", e);
                return Err(ServerError::Io(e));
            }
            
            eprintln!(
                "Cache saved successfully ({} items).",
                combined_cache_data.len()
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("Warning: Failed to encode data for cache: {}", e);
            Err(ServerError::Config(format!("Failed to encode cache data: {}", e)))
        }
    }
}

// Load or generate embeddings for a single crate
async fn load_or_generate_embeddings(
    workspace_path: &Path,
    crate_name: &str,
    features: &Option<Vec<String>>,
    openai_client: &OpenAIClient<OpenAIConfig>,
) -> Result<(Vec<Document>, Vec<(String, Array1<f32>)>, bool, Option<usize>, Option<f64>), ServerError> {
    // Generate a stable hash for the features
    let features_hash = hash_features(features);
    let cache_file_path = get_cache_file_path(crate_name, &features_hash)?;
    
    eprintln!("Cache file path for {}: {:?}", crate_name, cache_file_path);
    
    // Always load the current documentation first
    eprintln!("Loading docs for crate: {} (Features: {:?})", crate_name, features);
    let loaded_documents = doc_loader::load_documents(workspace_path, crate_name, features.as_ref())?;
    eprintln!("Loaded {} documents for {}.", loaded_documents.len(), crate_name);
    
    // Try loading embeddings from cache
    if cache_file_path.exists() {
        eprintln!("Attempting to load cached embeddings for {} from: {:?}", crate_name, cache_file_path);
        match File::open(&cache_file_path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match bincode::decode_from_reader::<Vec<CachedDocumentEmbedding>, _, _>(
                    reader,
                    config::standard(),
                ) {
                    Ok(cached_data) => {
                        eprintln!(
                            "Successfully loaded {} items from cache for {}.",
                            cached_data.len(), crate_name
                        );
                        
                        // Create a map of document paths to documents for easy lookup
                        let doc_map: std::collections::HashMap<String, &Document> = 
                            loaded_documents.iter().map(|doc| (doc.path.clone(), doc)).collect();
                        
                        // Check which documents can reuse cached embeddings and which need regeneration
                        let mut embeddings = Vec::new();
                        let mut documents_needing_embedding = Vec::new();
                        let mut reused_count = 0;
                        
                        // Create a set of paths for quick lookups
                        let mut cached_paths = std::collections::HashSet::new();
                        
                        // Process cached items first
                        for cached_item in &cached_data {
                            cached_paths.insert(cached_item.path.clone());
                            
                            // Check if this document still exists and content hash matches
                            if let Some(current_doc) = doc_map.get(&cached_item.path) {
                                if embeddings::content_hash_matches(cached_item, &current_doc.content) {
                                    // Content hash matches, reuse embedding
                                    embeddings.push((cached_item.path.clone(), Array1::from(cached_item.vector.clone())));
                                    reused_count += 1;
                                } else {
                                    // Content changed, needs new embedding
                                    documents_needing_embedding.push((*current_doc).clone());
                                }
                            }
                            // If document no longer exists, skip it
                        }
                        
                        // Add any new documents that weren't in the cache
                        for doc in &loaded_documents {
                            if !cached_paths.contains(&doc.path) {
                                documents_needing_embedding.push(doc.clone());
                            }
                        }
                        
                        eprintln!(
                            "Reusing {} cached embeddings, regenerating {} that changed or are new.",
                            reused_count, documents_needing_embedding.len()
                        );
                        
                        // If all documents can reuse embeddings, return early
                        if documents_needing_embedding.is_empty() {
                            return Ok((loaded_documents, embeddings, true, None, None));
                        }
                        
                        // Otherwise, continue with partial regeneration
                        if !documents_needing_embedding.is_empty() {
                            eprintln!("Generating embeddings for {} documents that changed or are new...", 
                                documents_needing_embedding.len());
                            
                            // Generate embeddings only for changed documents
                            let embedding_model: String = env::var("EMBEDDING_MODEL")
                                .unwrap_or_else(|_| "text-embedding-3-small".to_string());
                            
                            let (new_embeddings, tokens_used) = 
                                generate_embeddings(openai_client, &documents_needing_embedding, &embedding_model).await?;
                            
                            // Merge with reused embeddings
                            embeddings.extend(new_embeddings);
                            
                            // Calculate cost for the new embeddings
                            let cost_per_million = 0.02;
                            let estimated_cost = (tokens_used as f64 / 1_000_000.0) * cost_per_million;
                            
                            // We need to save the updated cache with all embeddings
                            eprintln!("Saving updated cache with merged embeddings...");
                            save_embeddings_to_cache(&cache_file_path, &loaded_documents, &embeddings)?;
                            
                            return Ok((loaded_documents, embeddings, false, Some(tokens_used), Some(estimated_cost)));
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to decode cache file for {}: {}. Will regenerate all embeddings.", crate_name, e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to open cache file for {}: {}. Will regenerate all embeddings.", crate_name, e);
            }
        }
    } else {
        eprintln!("Cache file not found for {}. Will generate all embeddings.", crate_name);
    }
    
    // Generate embeddings for all documents
    let embedding_model: String = env::var("EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());
    
    eprintln!("Generating embeddings for {} documents...", loaded_documents.len());
    let (generated_embeddings, total_tokens) =
        generate_embeddings(openai_client, &loaded_documents, &embedding_model).await?;
    
    // Calculate cost
    let cost_per_million = 0.02; // Cost per million tokens for the embedding model
    let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
    eprintln!(
        "Embedding generation cost for {} ({} tokens): ${:.6}",
        crate_name, total_tokens, estimated_cost
    );
    
    // Save to cache
    eprintln!("Saving generated embeddings for {} to cache.", crate_name);
    save_embeddings_to_cache(&cache_file_path, &loaded_documents, &generated_embeddings)?;
    
    Ok((loaded_documents, generated_embeddings, false, Some(total_tokens), Some(estimated_cost)))
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Parse CLI Arguments
    let cli = Cli::parse();
    
    if cli.crate_names.is_empty() {
        return Err(ServerError::Config("At least one crate name must be specified.".to_string()));
    }
    
    eprintln!("Crates to load: {:?}, Features: {:?}", cli.crate_names, cli.features);
    
    // Get absolute path for workspace
    let workspace_path = fs::canonicalize(&cli.workspace_path).map_err(|e| {
        ServerError::Config(format!(
            "Failed to resolve workspace path '{}': {}",
            cli.workspace_path.display(), e
        ))
    })?;
    
    eprintln!("Using workspace path: {}", workspace_path.display());
    
    // Check if target/doc exists
    let target_doc_path = workspace_path.join("target").join("doc");
    if !target_doc_path.exists() {
        return Err(ServerError::Config(format!(
            "Documentation directory not found at {}. Please run cargo doc before starting the server.",
            target_doc_path.display()
        )));
    }
    
    // Initialize OpenAI Client
    let _openai_api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?;
    
    let openai_client = if let Ok(api_base) = env::var("OPENAI_API_BASE") {
        let config = OpenAIConfig::new().with_api_base(api_base);
        OpenAIClient::with_config(config)
    } else {
        OpenAIClient::new()
    };
    
    // Set the OPENAI_CLIENT for embeddings
    OPENAI_CLIENT
        .set(openai_client.clone())
        .expect("Failed to set OpenAI client");
    
    // Initialize the server
    let startup_message = format!(
        "Rust Docs MCP Server initialized with {} crates. Use the query_rust_docs tool to query documentation.",
        cli.crate_names.len()
    );
    
    let service = RustDocsServer::new(startup_message)?;
    
    // Process each crate
    for crate_name in &cli.crate_names {
        let trimmed_name = crate_name.trim();
        eprintln!("Processing crate: {}", trimmed_name);
        
        // Load or generate embeddings for this crate
        let (documents, embeddings, loaded_from_cache, tokens, cost) = 
            load_or_generate_embeddings(&workspace_path, trimmed_name, &cli.features, &openai_client).await?;
        
        // Add the crate to the server
        let documents_len = documents.len();
        let embeddings_len = embeddings.len();
        
        service.add_crate(
            trimmed_name.to_string(),
            documents.clone(),
            embeddings.clone(),
        ).await?;
        
        // Log status
        if loaded_from_cache {
            eprintln!("Added crate '{}' from cache with {} documents.", 
                trimmed_name, documents_len);
        } else {
            eprintln!("Added crate '{}' with {} documents. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).", 
                trimmed_name, documents_len, embeddings_len, tokens.unwrap_or(0), cost.unwrap_or(0.0));
        }
    }
    
    // Start MCP server
    eprintln!("Rust Docs MCP server starting on stdio...");
    
    let server_handle = service.serve(stdio()).await.map_err(|e| {
        eprintln!("Failed to start server: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;
    
    eprintln!("Rust Docs MCP server running with crates: {:?}", cli.crate_names);
    
    // Wait for the server to complete
    server_handle.waiting().await.map_err(|e| {
        eprintln!("Server encountered an error while running: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;
    
    eprintln!("Rust Docs MCP server stopped.");
    Ok(())
}
