// Declare modules
mod doc_loader;
mod document_chunker;
mod embeddings;
mod embedding_cache_service;
mod error;
mod server;

// Test module
#[cfg(test)]
mod tests;

// Use necessary items from modules and crates
use crate::{
    doc_loader::Document,
    embeddings::{generate_embeddings, CachedDocumentEmbedding, Embedding, EmbeddingProvider, OPENAI_CLIENT},
    error::ServerError,
    server::RustDocsServer,
};
use async_openai::{Client as OpenAIClient, config::OpenAIConfig};
use bincode::config;
use cargo::core::PackageIdSpec;
use clap::Parser;
// Import rmcp items needed for the new approach
use rmcp::{
    transport::io::stdio, // Use the standard stdio transport
    ServiceExt,           // Import the ServiceExt trait for .serve() and .waiting()
};
use std::{
    collections::hash_map::DefaultHasher,
    env,
    fs::{self, File},
    hash::{Hash, Hasher}, // Import hashing utilities
    io::BufReader,
    path::PathBuf,
};
#[cfg(not(target_os = "windows"))]
use xdg::BaseDirectories;

// --- CLI Argument Parsing ---

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The package ID specification (e.g., "serde@^1.0", "tokio").
    #[arg()] // Positional argument
    package_spec: String,

    /// Optional features to enable for the crate when generating documentation.
    #[arg(short = 'F', long, value_delimiter = ',', num_args = 0..)] // Allow multiple comma-separated values
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
        .unwrap_or_else(|| "no_features".to_string()) // Use a specific string if no features
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loaded environment from: {}", path.display());
    }

    // --- Parse CLI Arguments ---
    let cli = Cli::parse();
    let specid_str = cli.package_spec.trim().to_string(); // Trim whitespace
    let features = cli.features.map(|f| {
        f.into_iter().map(|s| s.trim().to_string()).collect() // Trim each feature
    });

    // Parse the specid string
    let spec = PackageIdSpec::parse(&specid_str).map_err(|e| {
        ServerError::Config(format!(
            "Failed to parse package ID spec '{}': {}",
            specid_str, e
        ))
    })?;

    let crate_name = spec.name().to_string();
    let crate_version_req = spec
        .version()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "*".to_string());

    eprintln!(
        "Target Spec: {}, Parsed Name: {}, Version Req: {}, Features: {:?}",
        specid_str, crate_name, crate_version_req, features
    );

    // --- Determine Paths (incorporating features) ---

    // Sanitize the version requirement string
    let sanitized_version_req = crate_version_req
        .replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");

    // Generate a stable hash for the features to use in the path
    let features_hash = hash_features(&features);

    // Construct the relative path component including features hash
    let embeddings_relative_path = PathBuf::from(&crate_name)
        .join(&sanitized_version_req)
        .join(&features_hash) // Add features hash as a directory level
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
        // Ensure the base app cache directory exists
        fs::create_dir_all(&app_cache_dir).map_err(ServerError::Io)?;
        app_cache_dir.join(embeddings_relative_path)
    };

    eprintln!("Cache file path: {:?}", embeddings_file_path);

    // --- Try Loading Embeddings and Documents from Cache ---
    let mut loaded_from_cache = false;
    let mut loaded_embeddings: Option<Vec<(String, Embedding)>> = None;
    let mut loaded_documents_from_cache: Option<Vec<Document>> = None;

    if embeddings_file_path.exists() {
        eprintln!(
            "Attempting to load cached data from: {:?}",
            embeddings_file_path
        );
        match File::open(&embeddings_file_path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match bincode::decode_from_reader::<Vec<CachedDocumentEmbedding>, _, _>(
                    reader,
                    config::standard(),
                ) {
                    Ok(cached_data) => {
                        eprintln!(
                            "Successfully loaded {} items from cache. Separating data...",
                            cached_data.len()
                        );
                        let mut embeddings = Vec::with_capacity(cached_data.len());
                        let mut documents = Vec::with_capacity(cached_data.len());
                        for item in cached_data {
                            // Convert vector to Embedding struct for compatibility
                            let embedding = Embedding::new(
                                item.vector.clone(),
                                EmbeddingProvider::OpenAI, // Assume OpenAI as default provider for cached data
                                "text-embedding-3-small".to_string(), // Use default model name for cached data
                            );
                            embeddings.push((item.path.clone(), embedding));
                            documents.push(Document {
                                path: item.path,
                                content: item.content,
                            });
                        }
                        loaded_embeddings = Some(embeddings);
                        loaded_documents_from_cache = Some(documents);
                        loaded_from_cache = true;
                    }
                    Err(e) => {
                        eprintln!("Failed to decode cache file: {}. Will regenerate.", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to open cache file: {}. Will regenerate.", e);
            }
        }
    } else {
        eprintln!("Cache file not found. Will generate.");
    }

    // --- Generate or Use Loaded Embeddings ---
    let mut generated_tokens: Option<usize> = None;
    let mut generation_cost: Option<f64> = None;
    let mut documents_for_server: Vec<Document> = loaded_documents_from_cache.unwrap_or_default();

    // --- Initialize OpenAI Client (needed for question embedding even if cache hit) ---
    let openai_client = if let Ok(api_base) = env::var("OPENAI_API_BASE") {
        let config = OpenAIConfig::new().with_api_base(api_base);
        OpenAIClient::with_config(config)
    } else {
        OpenAIClient::new()
    };
    OPENAI_CLIENT
        .set(openai_client.clone()) // Clone the client for the OnceCell
        .expect("Failed to set OpenAI client");

    let final_embeddings = match loaded_embeddings {
        Some(embeddings) => {
            eprintln!("Using embeddings and documents loaded from cache.");
            embeddings
        }
        None => {
            eprintln!("Proceeding with documentation loading and embedding generation.");

            let _openai_api_key = env::var("OPENAI_API_KEY")
                .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?;

            eprintln!(
                "Loading documents for crate: {} (Version Req: {}, Features: {:?})",
                crate_name, crate_version_req, features
            );
            // Pass features to load_documents
            let loaded_documents =
                doc_loader::load_documents(&crate_name, &crate_version_req, features.as_ref())?; // Pass features here
            eprintln!("Loaded {} documents.", loaded_documents.len());
            documents_for_server = loaded_documents.clone();

            eprintln!("Generating embeddings...");
            let embedding_model: String = env::var("EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string());
            let (generated_embeddings, total_tokens) =
                generate_embeddings(&openai_client, &loaded_documents, &embedding_model).await?;

            let cost_per_million = 0.02;
            let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
            eprintln!(
                "Embedding generation cost for {} tokens: ${:.6}",
                total_tokens, estimated_cost
            );
            generated_tokens = Some(total_tokens);
            generation_cost = Some(estimated_cost);

            eprintln!(
                "Saving generated documents and embeddings to: {:?}",
                embeddings_file_path
            );

            let mut combined_cache_data: Vec<CachedDocumentEmbedding> = Vec::new();
            let embedding_map: std::collections::HashMap<String, Embedding> =
                generated_embeddings.clone().into_iter().collect();

            for doc in &loaded_documents {
                if let Some(embedding) = embedding_map.get(&doc.path) {
                    combined_cache_data.push(CachedDocumentEmbedding {
                        path: doc.path.clone(),
                        content: doc.content.clone(),
                        vector: embedding.values.clone(), // Extract vector values from Embedding struct
                    });
                } else {
                    eprintln!(
                        "Warning: Embedding not found for document path: {}. Skipping from cache.",
                        doc.path
                    );
                }
            }

            match bincode::encode_to_vec(&combined_cache_data, config::standard()) {
                Ok(encoded_bytes) => {
                    if let Some(parent_dir) = embeddings_file_path.parent() {
                        if !parent_dir.exists() {
                            if let Err(e) = fs::create_dir_all(parent_dir) {
                                eprintln!(
                                    "Warning: Failed to create cache directory {}: {}",
                                    parent_dir.display(),
                                    e
                                );
                            }
                        }
                    }
                    if let Err(e) = fs::write(&embeddings_file_path, encoded_bytes) {
                        eprintln!("Warning: Failed to write cache file: {}", e);
                    } else {
                        eprintln!(
                            "Cache saved successfully ({} items).",
                            combined_cache_data.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to encode data for cache: {}", e);
                }
            }
            generated_embeddings
        }
    };

    // --- Initialize and Start Server ---
    eprintln!(
        "Initializing server for crate: {} (Version Req: {}, Features: {:?})",
        crate_name, crate_version_req, features
    );

    let features_str = features
        .as_ref()
        .map(|f| format!(" Features: {:?}", f))
        .unwrap_or_default();

    let startup_message = if loaded_from_cache {
        format!(
            "Server for crate '{}' (Version Req: '{}'{}) initialized. Loaded {} embeddings from cache.",
            crate_name, crate_version_req, features_str, final_embeddings.len()
        )
    } else {
        let tokens = generated_tokens.unwrap_or(0);
        let cost = generation_cost.unwrap_or(0.0);
        format!(
            "Server for crate '{}' (Version Req: '{}'{}) initialized. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).",
            crate_name,
            crate_version_req,
            features_str,
            final_embeddings.len(),
            tokens,
            cost
        )
    };

    // Create the service instance using the updated ::new()
    let service = RustDocsServer::new(
        crate_name.clone(), // Pass crate_name directly
        documents_for_server,
        final_embeddings,
        startup_message,
    )?;

    // --- Use standard stdio transport and ServiceExt ---
    eprintln!("Rust Docs MCP server starting via stdio...");

    // Serve the server using the ServiceExt trait and standard stdio transport
    let server_handle = service.serve(stdio()).await.map_err(|e| {
        eprintln!("Failed to start server: {:?}", e);
        ServerError::McpRuntime(e.to_string()) // Use the new McpRuntime variant
    })?;

    eprintln!("{} Docs MCP server running...", &crate_name);

    // Wait for the server to complete (e.g., stdin closed)
    server_handle.waiting().await.map_err(|e| {
        eprintln!("Server encountered an error while running: {:?}", e);
        ServerError::McpRuntime(e.to_string()) // Use the new McpRuntime variant
    })?;

    eprintln!("Rust Docs MCP server stopped.");
    Ok(())
}
