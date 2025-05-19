// Declare modules
mod doc_loader;
mod document_chunker;
mod embedding_cache_service;
mod embeddings;
mod error;
mod server;

// Test module
#[cfg(test)]
mod tests;

// Use necessary items from modules and crates
use crate::{
    embeddings::OPENAI_CLIENT,
    error::ServerError,
    server::RustDocsServer,
};
use async_openai::{Client as OpenAIClient, config::OpenAIConfig};
use clap::Parser;
// Import rmcp items needed for the new approach
use rmcp::{
    ServiceExt,           // Import the ServiceExt trait for .serve() and .waiting()
    transport::io::stdio, // Use the standard stdio transport
};
use std::{
    collections::hash_map::DefaultHasher,
    env,
    hash::{Hash, Hasher},
};

// --- CLI Argument Parsing ---

#[derive(Parser, Debug)]
#[command(author, version, about = "MCP server for querying Rust crate documentation", long_about = None)]
struct Cli {
    // No required arguments - server will use locally available crate docs
}

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    if let Ok(path) = dotenvy::dotenv() {
        eprintln!("Loaded environment from: {}", path.display());
    }

    // Parse CLI Arguments - now just a simple parse with no required args
    let _cli = Cli::parse();

    // Initialize OpenAI Client
    let openai_client = if let Ok(api_base) = env::var("OPENAI_API_BASE") {
        let config = OpenAIConfig::new().with_api_base(api_base);
        OpenAIClient::with_config(config)
    } else {
        OpenAIClient::new()
    };
    OPENAI_CLIENT
        .set(openai_client.clone())
        .expect("Failed to set OpenAI client");

    // Check if the target/doc directory exists
    let target_doc_path = std::path::Path::new("./target/doc");
    if !target_doc_path.exists() {
        eprintln!(
            "Warning: ./target/doc directory not found. Run 'cargo doc' to generate documentation for local crates."
        );
    }

    // Create a simple startup message
    let startup_message = "Rust Docs MCP server initialized. Use the query_rust_docs tool to query documentation for any crate that has been generated with 'cargo doc'.".to_string();

    // Create the service instance with simplified constructor
    let service = RustDocsServer::new(startup_message)?;

    // Start the server via stdio
    eprintln!("Rust Docs MCP server starting via stdio...");

    // Serve the server
    let server_handle = service.serve(stdio()).await.map_err(|e| {
        eprintln!("Failed to start server: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;

    eprintln!("Rust Docs MCP server running...");

    // Wait for the server to complete
    server_handle.waiting().await.map_err(|e| {
        eprintln!("Server encountered an error while running: {:?}", e);
        ServerError::McpRuntime(e.to_string())
    })?;

    eprintln!("Rust Docs MCP server stopped.");
    Ok(())
}
