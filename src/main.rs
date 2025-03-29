// Declare modules (keep doc_loader, embeddings, error)
mod doc_loader;
mod embeddings;
mod error;
mod server; // Keep server module as RustDocsServer is defined there

// Use necessary items from modules and crates
use crate::{
    doc_loader::Document,
    embeddings::{generate_embeddings, CachedDocumentEmbedding, OPENAI_CLIENT},
    error::ServerError,
    server::RustDocsServer, // Import the updated RustDocsServer
};
use async_openai::Client as OpenAIClient;
use bincode::config;
use ndarray::Array1;
// Import rmcp items needed for the new approach
use cargo::core::PackageIdSpec;

use rmcp::{
    transport::io::stdio, // Use the standard stdio transport
    ServiceExt,           // Import the ServiceExt trait for .serve() and .waiting()
};
use std::{
    env,
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
};
#[cfg(not(target_os = "windows"))]
use xdg::BaseDirectories;

// No changes needed below this line until server initialization/running

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Initialize tracing (optional, but good practice)
    // Consider adding tracing_subscriber setup here if not already present elsewhere
    // Example:
    // tracing_subscriber::fmt()
    //     .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    //     .init();
    // eprintln!("Tracing initialized."); // Use eprintln for startup messages

    // Get Package ID Spec from command line argument
    let mut args = env::args().skip(1); // Skip program name
    let specid_str = args.next().ok_or_else(|| {
        eprintln!("Usage: rustdocs_mcp_server <PACKAGE_ID_SPEC>");
        eprintln!("Example: rustdocs_mcp_server \"serde@^1.0\"");
        ServerError::MissingArgument("PACKAGE_ID_SPEC".to_string())
    })?;

    // Parse the specid string
    let spec = PackageIdSpec::parse(&specid_str).map_err(|e| {
        ServerError::Config(format!(
            "Failed to parse package ID spec '{}': {}",
            specid_str, e
        ))
    })?;

    let crate_name = spec.name().to_string();
    // Use '*' as default version requirement if not specified in the spec
    // Corrected method name: version() instead of version_req()
    let crate_version_req = spec.version().map(|v| v.to_string()).unwrap_or_else(|| "*".to_string());


    eprintln!("Target Spec: {}, Parsed Name: {}, Version Req: {}", specid_str, crate_name, crate_version_req); // Use eprintln

    // --- Determine Paths ---

    // Sanitize the version requirement string for use in the path (needed for both paths)
    let sanitized_version_req = crate_version_req.replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");

    // Construct the relative path component (needed for both paths)
    let embeddings_relative_path = PathBuf::from(&crate_name)
        .join(&sanitized_version_req) // Add sanitized version req as a directory
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


    eprintln!("Cache file path: {:?}", embeddings_file_path); // Use eprintln

    // --- Try Loading Embeddings and Documents from Cache ---
    let mut loaded_from_cache = false;
    let mut loaded_embeddings: Option<Vec<(String, Array1<f32>)>> = None;
    let mut loaded_documents_from_cache: Option<Vec<Document>> = None;

    if embeddings_file_path.exists() {
        eprintln!(
            "Attempting to load cached data from: {:?}",
            embeddings_file_path
        ); // Use eprintln
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
                        ); // Use eprintln
                        let mut embeddings = Vec::with_capacity(cached_data.len());
                        let mut documents = Vec::with_capacity(cached_data.len());
                        for item in cached_data {
                            embeddings.push((item.path.clone(), Array1::from(item.vector)));
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
                        eprintln!( // Use eprintln
                            "Failed to decode cache file: {}. Will regenerate.",
                            e
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!( // Use eprintln
                    "Failed to open cache file: {}. Will regenerate.",
                    e
                );
            }
        }
    } else {
        eprintln!("Cache file not found. Will generate."); // Use eprintln
    }

    // --- Generate or Use Loaded Embeddings ---
    let mut generated_tokens: Option<usize> = None;
    let mut generation_cost: Option<f64> = None;
    let mut documents_for_server: Vec<Document> = loaded_documents_from_cache.unwrap_or_default();

    // --- Initialize OpenAI Client (needed for question embedding even if cache hit) ---
    let openai_client = OpenAIClient::new();
    OPENAI_CLIENT
        .set(openai_client.clone()) // Clone the client for the OnceCell
        .expect("Failed to set OpenAI client");

    let final_embeddings = match loaded_embeddings {
        Some(embeddings) => {
            eprintln!("Using embeddings and documents loaded from cache."); // Use eprintln
            embeddings
        }
        None => {
            eprintln!("Proceeding with documentation loading and embedding generation."); // Use eprintln

            let _openai_api_key = env::var("OPENAI_API_KEY")
                .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?;

            // Client initialization moved earlier

            eprintln!("Loading documents for crate: {} (Version Req: {})", crate_name, crate_version_req); // Use eprintln
            // Use crate_name and crate_version_req here
            let loaded_documents = doc_loader::load_documents(&crate_name, &crate_version_req)?;
            eprintln!("Loaded {} documents.", loaded_documents.len()); // Use eprintln
            documents_for_server = loaded_documents.clone();

            eprintln!("Generating embeddings..."); // Use eprintln
            let (generated_embeddings, total_tokens) = generate_embeddings(
                &openai_client,
                &loaded_documents,
                "text-embedding-3-small",
            )
            .await?;

            let cost_per_million = 0.02;
            let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
            eprintln!( // Use eprintln
                "Embedding generation cost for {} tokens: ${:.6}",
                total_tokens, estimated_cost
            );
            generated_tokens = Some(total_tokens);
            generation_cost = Some(estimated_cost);

            eprintln!( // Use eprintln
                "Saving generated documents and embeddings to: {:?}",
                embeddings_file_path
            );

            let mut combined_cache_data: Vec<CachedDocumentEmbedding> = Vec::new();
            let embedding_map: std::collections::HashMap<String, Array1<f32>> =
                generated_embeddings.clone().into_iter().collect();

            for doc in &loaded_documents {
                if let Some(embedding_array) = embedding_map.get(&doc.path) {
                    combined_cache_data.push(CachedDocumentEmbedding {
                        path: doc.path.clone(),
                        content: doc.content.clone(),
                        vector: embedding_array.to_vec(),
                    });
                } else {
                    eprintln!( // Use eprintln
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
                                eprintln!( // Use eprintln
                                    "Warning: Failed to create cache directory {}: {}",
                                    parent_dir.display(),
                                    e
                                );
                            }
                        }
                    }
                    if let Err(e) = fs::write(&embeddings_file_path, encoded_bytes) {
                        eprintln!("Warning: Failed to write cache file: {}", e); // Use eprintln
                    } else {
                        eprintln!( // Use eprintln
                            "Cache saved successfully ({} items).",
                            combined_cache_data.len()
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to encode data for cache: {}", e); // Use eprintln
                }
            }
            generated_embeddings
        }
    };

    // --- Initialize and Start Server ---
    eprintln!("Initializing server for crate: {}", crate_name); // Use eprintln

    let startup_message = if loaded_from_cache {
        format!(
            "Server for crate '{}' (Version Req: '{}') initialized. Loaded {} embeddings from cache.",
            crate_name,
            crate_version_req, // Add version req here
            final_embeddings.len()
        )
    } else {
        let tokens = generated_tokens.unwrap_or(0);
        let cost = generation_cost.unwrap_or(0.0);
        format!(
            "Server for crate '{}' (Version Req: '{}') initialized. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).",
            crate_name,
            crate_version_req, // Add version req here
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
    eprintln!("Rust Docs MCP server starting via stdio..."); // Use eprintln

    // Serve the server using the ServiceExt trait and standard stdio transport
    let server_handle = service.serve(stdio()).await.map_err(|e| {
        eprintln!("Failed to start server: {:?}", e); // Use eprintln
        ServerError::McpRuntime(e.to_string()) // Use the new McpRuntime variant
    })?;

    eprintln!("{} Docs MCP server running...", &crate_name); // Use eprintln

    // Wait for the server to complete (e.g., stdin closed)
    server_handle.waiting().await.map_err(|e| {
        eprintln!("Server encountered an error while running: {:?}", e); // Use eprintln
        ServerError::McpRuntime(e.to_string()) // Use the new McpRuntime variant
    })?;

    eprintln!("Rust Docs MCP server stopped."); // Use eprintln
    Ok(())
}
