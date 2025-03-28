// Declare modules
mod doc_loader;
mod embeddings;
mod error;
mod server;
mod service;
mod state;
mod transport; // Declare transport module

// Use necessary items from modules and crates
use crate::{
    doc_loader::{Document}, // Import Document struct and module
    embeddings::{generate_embeddings, SerializableEmbedding, OPENAI_CLIENT}, // Group imports
    error::ServerError,
    server::RustDocsServer,
    transport::StdioTransport,
};
use async_openai::Client as OpenAIClient;
use bincode::config; // Keep config
use ndarray::Array1;
use rmcp::{serve_server, transport::io::stdio};
use std::{
    env,
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
};
use xdg::BaseDirectories;

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Get crate name and version from command line arguments
    let mut args = env::args().skip(1); // Skip program name
    let crate_name = args.next().ok_or_else(|| {
        eprintln!("Usage: rustdocs_mcp_server <CRATE_NAME> <CRATE_VERSION>");
        ServerError::MissingArgument("CRATE_NAME".to_string())
    })?;
    let crate_version = args.next().ok_or_else(|| {
        eprintln!("Usage: rustdocs_mcp_server <CRATE_NAME> <CRATE_VERSION>");
        ServerError::MissingArgument("CRATE_VERSION".to_string())
    })?;

    // --- Determine Paths ---
    let xdg_dirs = BaseDirectories::with_prefix("rustdocs-mcp-server")
        .map_err(|e| ServerError::Xdg(format!("Failed to get XDG directories: {}", e)))?;

    // Construct the path for embeddings file
    let embeddings_relative_path = PathBuf::from(&crate_name).join("embeddings.bin");
    let embeddings_file_path = xdg_dirs
        .place_data_file(embeddings_relative_path)
        .map_err(ServerError::Io)?;

    // --- Try Loading Embeddings ---
    let mut loaded_from_cache = false;
    let mut loaded_embeddings: Option<Vec<(String, Array1<f32>)>> = None;

    if embeddings_file_path.exists() {
        println!("Attempting to load embeddings from: {:?}", embeddings_file_path);
        match File::open(&embeddings_file_path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                match bincode::decode_from_reader::<Vec<SerializableEmbedding>, _, _>(
                    reader,
                    config::standard(),
                ) {
                    Ok(loaded_serializable) => {
                        println!("Successfully loaded embeddings from cache. Converting format...");
                        let converted = loaded_serializable
                            .into_iter()
                            .map(|se| (se.path, Array1::from(se.vector)))
                            .collect::<Vec<_>>();
                        loaded_embeddings = Some(converted);
                        loaded_from_cache = true; // Set flag
                    }
                    Err(e) => {
                        println!(
                            "Failed to decode embeddings file: {}. Will regenerate.",
                            e
                        );
                        // Proceed to generation
                    }
                }
            }
            Err(e) => {
                println!(
                    "Failed to open embeddings file: {}. Will regenerate.",
                    e
                );
                // Proceed to generation
            }
        }
    } else {
        println!("Embeddings file not found. Will generate.");
        // Proceed to generation
    }

    // --- Generate or Use Loaded Embeddings ---
    let mut generated_tokens: Option<usize> = None;
    let mut generation_cost: Option<f64> = None;
    let mut documents_for_server: Vec<Document> = Vec::new(); // Empty by default

    let final_embeddings = match loaded_embeddings {
        Some(embeddings) => {
            println!("Using embeddings loaded from cache.");
            embeddings // Use the ones loaded from the file
        }
        None => {
            // --- Generation Path ---
            println!("Proceeding with documentation loading and embedding generation.");

            // Ensure OpenAI API key is available ONLY if generating
            let _openai_api_key = env::var("OPENAI_API_KEY")
                .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?;

            // Initialize OpenAI client ONLY if generating
            let openai_client = OpenAIClient::new();
            OPENAI_CLIENT
                .set(openai_client.clone())
                .expect("Failed to set OpenAI client");

            // 1. Load documents
            println!("Loading documents for crate: {}", crate_name);
            // Use the imported module function directly
            let loaded_documents = doc_loader::load_documents(&crate_name, &crate_version)?;
            println!("Loaded {} documents.", loaded_documents.len());
            documents_for_server = loaded_documents.clone(); // Clone for server if needed (though user said no)

            // 2. Generate embeddings
            println!("Generating embeddings...");
            let (generated_embeddings, total_tokens) = generate_embeddings(
                &openai_client,
                &loaded_documents, // Use the just-loaded documents
                "text-embedding-3-small",
            )
            .await?;

            // Calculate and store cost
            let cost_per_million = 0.02;
            let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
            println!(
                "Embedding generation cost for {} tokens: ${:.6}",
                total_tokens, estimated_cost
            );
            generated_tokens = Some(total_tokens);
            generation_cost = Some(estimated_cost);

            // 3. Save embeddings
            println!("Saving generated embeddings to: {:?}", embeddings_file_path);
            let serializable_embeddings: Vec<SerializableEmbedding> = generated_embeddings
                .iter()
                .map(|(path, array)| SerializableEmbedding {
                    path: path.clone(),
                    vector: array.to_vec(),
                })
                .collect();

            match bincode::encode_to_vec(&serializable_embeddings, config::standard()) {
                Ok(encoded_bytes) => {
                    if let Err(e) = fs::write(&embeddings_file_path, encoded_bytes) {
                        println!("Warning: Failed to write embeddings file: {}", e);
                    } else {
                        println!("Embeddings saved successfully.");
                    }
                }
                Err(e) => {
                    println!("Warning: Failed to encode embeddings to vec: {}", e);
                }
            }
            generated_embeddings // Return the generated embeddings
        }
    };

    // --- Initialize and Start Server ---
    println!("Initializing server for crate: {}", crate_name);

    // Prepare the startup summary message
    let startup_message = if loaded_from_cache {
        format!(
            "Server for crate '{}' initialized. Loaded {} embeddings from cache.",
            crate_name,
            final_embeddings.len() // Use count from loaded/generated embeddings
        )
    } else {
        // Embeddings were generated
        let tokens = generated_tokens.unwrap_or(0);
        let cost = generation_cost.unwrap_or(0.0);
        format!(
            "Server for crate '{}' initialized. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).",
            crate_name,
            final_embeddings.len(), // Use count from loaded/generated embeddings
            tokens,
            cost
        )
    };

    // Create the service instance
    // Pass the final embeddings and an empty Vec for documents as it's not needed by the service
    let service = RustDocsServer::new(
        crate_name,
        documents_for_server, // Pass the (potentially empty) documents vec
        final_embeddings,
        startup_message,
    )?;

    // Create the stdio transport
    let (stdin, stdout) = stdio();
    let transport = StdioTransport {
        reader: stdin,
        writer: stdout,
    };

    println!("Rust Docs MCP server starting...");

    // Serve the server
    serve_server(service, transport).await?;

    println!("Rust Docs MCP server stopped.");
    Ok(())
}
