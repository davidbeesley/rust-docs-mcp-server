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
    doc_generator::generate_docs_for_deps, embeddings::OPENAI_CLIENT, error::ServerError,
    server::RustDocsServer,
};
use async_openai::{Client as OpenAIClient, config::OpenAIConfig};
use clap::Parser;
use rmcp::{ServiceExt, transport::io::stdio};
use std::{env, fs, path::PathBuf};

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

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Parse CLI Arguments
    let cli = Cli::parse();

    // If crate names are provided, we'll preload them. Otherwise, we'll use lazy loading
    if !cli.crate_names.is_empty() {
        eprintln!(
            "Crates to preload: {:?}, Features: {:?}",
            cli.crate_names, cli.features
        );
    } else {
        eprintln!(
            "No crates specified for preloading. Will use lazy loading. Features: {:?}",
            cli.features
        );
    }

    // Get absolute path for workspace
    let workspace_path = fs::canonicalize(&cli.workspace_path).map_err(|e| {
        ServerError::Config(format!(
            "Failed to resolve workspace path '{}': {}",
            cli.workspace_path.display(),
            e
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

    // Determine the configuration information for the server
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

    // Create the server instance
    let server = RustDocsServer::new(
        config_info,
        workspace_path.clone(),
        cli.features.clone(),
        enable_lazy_loading,
    )?;

    // Preload crates if needed
    for crate_name in &cli.crate_names {
        let trimmed_name = crate_name.trim();
        eprintln!("Preloading crate: {}", trimmed_name);

        // Get the OpenAI client
        let openai_client = OPENAI_CLIENT
            .get()
            .ok_or_else(|| ServerError::Config("OpenAI client not initialized".to_string()))?;

        // Load or generate embeddings for this crate
        let (documents, embeddings, loaded_from_cache, tokens, cost) =
            server.load_crate_data(trimmed_name, openai_client).await?;

        // Add the crate to the server
        let documents_len = documents.len();
        let embeddings_len = embeddings.len();

        server
            .add_crate(trimmed_name.to_string(), documents, embeddings)
            .await?;

        // Log status
        if loaded_from_cache {
            eprintln!(
                "Preloaded crate '{}' from cache with {} documents.",
                trimmed_name, documents_len
            );
        } else {
            eprintln!(
                "Preloaded crate '{}' with {} documents. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).",
                trimmed_name,
                documents_len,
                embeddings_len,
                tokens.unwrap_or(0),
                cost.unwrap_or(0.0)
            );
        }
    }

    // Start MCP server
    eprintln!("Rust Docs MCP server starting on stdio...");

    // Start the server
    let server_handle = server.serve(stdio()).await.map_err(|e| {
        eprintln!("Failed to start server: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;

    eprintln!(
        "Rust Docs MCP server running with crates: {:?}",
        cli.crate_names
    );

    // Wait for the server to complete
    server_handle.waiting().await.map_err(|e| {
        eprintln!("Server encountered an error while running: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;

    eprintln!("Rust Docs MCP server stopped.");
    Ok(())
}
