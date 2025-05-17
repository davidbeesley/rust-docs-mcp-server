// Declare modules
mod crate_discovery;
mod doc_generator;
mod doc_loader;
mod embeddings;
mod error;
mod fast_hash;
mod global_cache;
mod server;

// Use necessary items from modules and crates
use crate::{
    doc_loader::Document,
    doc_generator::generate_docs_for_deps,
    embeddings::{generate_embeddings, OPENAI_CLIENT},
    error::ServerError,
    server::RustDocsServer,
};
use async_openai::{Client as OpenAIClient, config::OpenAIConfig};
use clap::Parser;
use ndarray::Array1;
use rmcp::{transport::io::stdio, ServiceExt};
use std::{
    env,
    fs,
    path::{Path, PathBuf},
};

// --- CLI Argument Parsing ---

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to workspace root directory (where target/doc is located)
    #[arg(short = 'w', long, default_value = ".")]
    workspace_path: PathBuf,
    
    /// Optional features enabled when the documentation was generated.
    #[arg(short = 'F', long, value_delimiter = ',', num_args = 0..)]
    features: Option<Vec<String>>,

    /// Generate documentation even if target/doc doesn't exist
    #[arg(short = 'g', long)]
    generate_docs: bool,

    /// Disable lazy loading for crates not explicitly named (only preloaded crates will be available)
    #[arg(short = 'p', long)]
    preload: bool,

    /// Crate names to preload documentation for at startup
    /// If specified without --preload: These crates will be preloaded, others lazily loaded when needed
    /// If specified with --preload: Only these crates will be available (others cannot be loaded)
    /// If not specified with --preload: All available crates will be preloaded
    #[arg(value_delimiter = ',')]
    crate_names: Vec<String>,
}

// No legacy caching functions needed anymore since we're using content-based caching
// The old functions for hash_features, get_cache_file_path and save_embeddings_to_cache have been removed

// Load or generate embeddings for a single crate
async fn load_or_generate_embeddings(
    workspace_path: &Path,
    crate_name: &str,
    features: &Option<Vec<String>>,
    openai_client: &OpenAIClient<OpenAIConfig>,
) -> Result<(Vec<Document>, Vec<(String, Array1<f32>)>, bool, Option<usize>, Option<f64>), ServerError> {
    // Always load the current documentation first
    eprintln!("Loading docs for crate: {} (Features: {:?})", crate_name, features);
    let loaded_documents = doc_loader::load_documents(workspace_path, crate_name)?;
    eprintln!("Loaded {} documents for {}.", loaded_documents.len(), crate_name);
    
    // Prepare storage for embeddings
    let mut embeddings = Vec::new();
    let mut documents_needing_embedding = Vec::new();
    let mut reused_count = 0;
    
    // Try to get embeddings from global content hash cache
    for doc in &loaded_documents {
        // Get document embedding from the global cache
        if let Some(embedding_vec) = embeddings::get_embedding_by_content(&doc.content) {
            // Found in global cache - reuse
            embeddings.push((doc.path.clone(), Array1::from(embedding_vec)));
            reused_count += 1;
        } else {
            // Not found in cache - needs embedding
            documents_needing_embedding.push(doc.clone());
        }
    }
    
    eprintln!(
        "Reusing {} cached embeddings, generating {} new embeddings.",
        reused_count, documents_needing_embedding.len()
    );
    
    // If all documents have cached embeddings, return early
    if documents_needing_embedding.is_empty() {
        return Ok((loaded_documents, embeddings, true, None, None));
    }
    
    // Generate embeddings for documents not in cache
    let embedding_model: String = env::var("EMBEDDING_MODEL")
        .unwrap_or_else(|_| "text-embedding-3-small".to_string());
    
    eprintln!("Generating embeddings for {} documents...", documents_needing_embedding.len());
    let (new_embeddings, total_tokens) =
        generate_embeddings(openai_client, &documents_needing_embedding, &embedding_model).await?;
    
    // Store new embeddings in global cache
    eprintln!("Storing {} new embeddings in global cache...", new_embeddings.len());
    for (i, (_path, embedding)) in new_embeddings.iter().enumerate() {
        if let Some(doc) = documents_needing_embedding.get(i) {
            // Store in global cache
            if let Err(e) = embeddings::store_embedding_by_content(
                &doc.content,
                &embedding.to_vec(),
            ) {
                eprintln!("Warning: Failed to store in global cache: {}", e);
            }
        }
    }
    
    // Calculate cost for the new embeddings
    let cost_per_million = 0.02;
    let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
    eprintln!(
        "Embedding generation cost for {} ({} tokens): ${:.6}",
        crate_name, total_tokens, estimated_cost
    );
    
    // Merge with reused embeddings
    embeddings.extend(new_embeddings);
    
    Ok((loaded_documents, embeddings, false, Some(total_tokens), Some(estimated_cost)))
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Parse CLI Arguments
    let cli = Cli::parse();
    
    // If crate names are provided, we'll preload them. Otherwise, we'll use lazy loading
    if !cli.crate_names.is_empty() {
        eprintln!("Crates to preload: {:?}, Features: {:?}", cli.crate_names, cli.features);
    } else {
        eprintln!("No crates specified for preloading. Will use lazy loading. Features: {:?}", cli.features);
    }
    
    // Get absolute path for workspace
    let workspace_path = fs::canonicalize(&cli.workspace_path).map_err(|e| {
        ServerError::Config(format!(
            "Failed to resolve workspace path '{}': {}",
            cli.workspace_path.display(), e
        ))
    })?;
    
    eprintln!("Using workspace path: {}", workspace_path.display());
    
    // Check if target/doc exists, generate if needed
    let target_doc_path = workspace_path.join("target").join("doc");
    
    if !target_doc_path.exists() {
        if cli.generate_docs {
            eprintln!("Documentation directory not found. Generating docs using Cargo.toml...");
            let cargo_toml_path = workspace_path.join("Cargo.toml");
            if !cargo_toml_path.exists() {
                return Err(ServerError::Config(format!(
                    "Cargo.toml not found at {}. Cannot generate documentation.",
                    cargo_toml_path.display()
                )));
            }
            
            // Generate docs
            let doc_path = generate_docs_for_deps(&cargo_toml_path, &cli.features)?;
            eprintln!("Documentation generated at: {}", doc_path.display());
        } else {
            return Err(ServerError::Config(format!(
                "Documentation directory not found at {}. Please run cargo doc before starting the server or use --generate-docs to generate documentation.",
                target_doc_path.display()
            )));
        }
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
    
    // By default, lazy loading is enabled unless preload flag is set
    let enable_lazy_loading = !cli.preload;
    
    // If specific crates were named, we'll preload just those
    // If preload flag is set with no specific crates, we'll load all available crates
    let config_info = if enable_lazy_loading && cli.crate_names.is_empty() {
        "Rust Docs MCP Server initialized with lazy loading enabled. Use the query_rust_docs tool to query documentation.".to_string()
    } else if !cli.crate_names.is_empty() {
        format!(
            "Rust Docs MCP Server initialized with {} specified crates preloaded. Use the query_rust_docs tool to query documentation.",
            cli.crate_names.len()
        )
    } else {
        "Rust Docs MCP Server initialized with all available crates preloaded. Use the query_rust_docs tool to query documentation.".to_string()
    };
    
    // Create server with the workspace path and lazy loading settings
    let service = RustDocsServer::new(
        config_info,
        workspace_path.clone(),
        cli.features.clone(),
        enable_lazy_loading,
    )?;
    
    // Determine which crates to preload
    let target_doc_path = workspace_path.join("target").join("doc");
    let crates_to_preload = if !cli.crate_names.is_empty() {
        // Preload only specific crates
        cli.crate_names.clone()
    } else if cli.preload {
        // Preload all available crates
        match crate_discovery::discover_available_crates(&target_doc_path) {
            Ok(crates) => {
                eprintln!("Found {} crates available to preload: {:?}", crates.len(), crates);
                crates
            },
            Err(e) => {
                eprintln!("Error discovering crates: {}", e);
                Vec::new()
            }
        }
    } else {
        // No preloading, just log available crates
        eprintln!("Using lazy loading. Will load crates as they are requested.");
        
        // Log available crates in target/doc
        match crate_discovery::discover_available_crates(&target_doc_path) {
            Ok(crates) => {
                eprintln!("Found {} crates available in {}: {:?}", 
                    crates.len(), target_doc_path.display(), crates);
            },
            Err(e) => {
                eprintln!("Error discovering crates: {}", e);
            }
        }
        Vec::new()
    };
    
    // Preload crates if needed
    for crate_name in &crates_to_preload {
        let trimmed_name = crate_name.trim();
        eprintln!("Preloading crate: {}", trimmed_name);
        
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
            eprintln!("Preloaded crate '{}' from cache with {} documents.", 
                trimmed_name, documents_len);
        } else {
            eprintln!("Preloaded crate '{}' with {} documents. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).", 
                trimmed_name, documents_len, embeddings_len, tokens.unwrap_or(0), cost.unwrap_or(0.0));
        }
    }
    
    // Start MCP server
    eprintln!("Rust Docs MCP server starting on stdio...");
    
    // Use Arc::new on the service
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
