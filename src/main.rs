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
    doc_loader::load_documents,
    embeddings::{generate_embeddings, OPENAI_CLIENT},
    embeddings::SerializableEmbedding,
    error::ServerError,
    server::RustDocsServer,
    transport::StdioTransport, // Import StdioTransport
};
use async_openai::Client as OpenAIClient;
use rmcp::{
    serve_server,
    transport::io::stdio, // Keep stdio function
    // Remove TransportAdapterAsyncCombinedRW import
};
use std::env;
use ndarray::Array1;
use std::fs::{self, File};
use std::io::BufReader; // Removed unused BufWriter
use std::path::PathBuf; // Removed unused Path
use xdg::BaseDirectories;
use bincode::{
    config,
    // serde::OwnedSerdeDecoder, // No longer needed
    // decode_from_reader, // Removed unused import
    // encode_to_vec, // Removed unused import
    // Encode, Decode, // No longer needed directly
};

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

    let _openai_api_key = env::var("OPENAI_API_KEY")
        .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?; // Needed later

    // Load documents by generating them dynamically
    println!("Loading documents for crate: {}", crate_name);
    let documents = load_documents(&crate_name, &crate_version)?; // Pass crate_name and crate_version
    println!("Loaded {} documents.", documents.len());

    // Initialize OpenAI client and set it in the OnceLock
    let openai_client = OpenAIClient::new();
    OPENAI_CLIENT
        .set(openai_client.clone()) // Clone for generate_embeddings
        .expect("Failed to set OpenAI client");

    // --- Persistence Logic ---
    // Use XDG Base Directory specification for data storage
    let xdg_dirs = BaseDirectories::with_prefix("rustdocs-mcp-server")
        .map_err(|e| ServerError::Xdg(format!("Failed to get XDG directories: {}", e)))?; // Use the new Xdg variant

    // Construct the path within the XDG data directory, including the crate name
    let relative_path = PathBuf::from(&crate_name).join("embeddings.bin");

    // Use place_data_file to get the full path and ensure parent directories exist
    let embeddings_file_path = xdg_dirs.place_data_file(relative_path)
        .map_err(ServerError::Io)?; // Map IO error if directory creation fails

    let embeddings = if embeddings_file_path.exists() {
        println!("Loading embeddings from: {:?}", embeddings_file_path);
        match File::open(&embeddings_file_path) {
            Ok(file) => {
                let reader = BufReader::new(file);
                // Use top-level decode_from_reader now that bincode serde feature is enabled
                match bincode::decode_from_reader::<Vec<SerializableEmbedding>, _, _>(reader, config::standard()) {
                    Ok(loaded_serializable) => {
                        println!("Successfully loaded embeddings. Converting format...");
                        // Convert back to Vec<(String, Array1<f32>)>
                        let converted_embeddings = loaded_serializable
                            .into_iter()
                            .map(|se| (se.path, Array1::from(se.vector))) // Convert Vec to Array1
                            .collect::<Vec<_>>();
                        Some(converted_embeddings) // Wrap in Option for the outer match
                    }
                    Err(e) => {
                        println!("Failed to decode embeddings: {}. Regenerating...", e);
                        // Fall through to regeneration
                        None
                    }
                }
            }
            Err(e) => {
                println!("Failed to open embeddings file: {}. Regenerating...", e);
                // Fall through to regeneration
                None
            }
        }
    } else {
        println!("Embeddings file not found. Generating...");
        None
    };

    // Use loaded embeddings or generate new ones if loading failed or file didn't exist
    // Variables to store generation stats if needed
    let mut generated_tokens: Option<usize> = None;
    let mut generation_cost: Option<f64> = None;

    let embeddings = match embeddings {
        Some(e) => e,
        None => {
            // Directory creation is handled by xdg_dirs.place_data_file

            // Generate embeddings
            println!("Generating embeddings...");
            // Capture the returned tuple (embeddings, total_tokens)
            let (generated_embeddings, total_tokens) =
                generate_embeddings(&openai_client, &documents, "text-embedding-3-small").await?;

            // Calculate and print cost
            // Price: $0.02 / 1M tokens for text-embedding-3-small
            let cost_per_million = 0.02;
            let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
            println!(
                "Embedding generation cost for {} tokens: ${:.6}", // Format for cents/fractions
                total_tokens, estimated_cost
            );
            // Store generation stats
            generated_tokens = Some(total_tokens);
            generation_cost = Some(estimated_cost);


            println!("Embeddings generated. Saving to: {:?}", embeddings_file_path);

            // Convert to serializable format
            let serializable_embeddings: Vec<SerializableEmbedding> = generated_embeddings // Use the embeddings from the tuple
                .iter()
                .map(|(path, array)| SerializableEmbedding {
                    path: path.clone(),
                    vector: array.to_vec(), // Convert Array1 to Vec
                })
                .collect();

            // Encode directly to Vec<u8>
            match bincode::encode_to_vec(&serializable_embeddings, config::standard()) {
                Ok(encoded_bytes) => {
                    // Write the bytes to the file
                    if let Err(e) = fs::write(&embeddings_file_path, encoded_bytes) {
                        println!("Warning: Failed to write embeddings file: {}", e);
                    } else {
                        println!("Embeddings saved successfully.");
                    }
                }
                Err(e) => {
                    // Log error but continue
                    println!("Warning: Failed to encode embeddings to vec: {}", e);
                }
            }
            generated_embeddings
        }
    };
    // --- End Persistence Logic ---


    println!("Initializing server for crate: {}", crate_name);

    // Create the service instance, passing embeddings
    // Prepare the startup summary message
    let startup_message = {
        let doc_count = documents.len();
        match (generated_tokens, generation_cost) {
            (Some(tokens), Some(cost)) => {
                // Embeddings were generated
                format!(
                    "Server for crate '{}' initialized. Loaded {} documents. Generated embeddings for {} tokens (Est. Cost: ${:.6}).",
                    crate_name, doc_count, tokens, cost
                )
            }
            _ => {
                // Embeddings were loaded from cache
                format!(
                    "Server for crate '{}' initialized. Loaded {} documents from cache.",
                    crate_name, doc_count
                )
            }
        }
    };

    // Note: We still pass 'documents' which were loaded regardless of embedding source
    let service = RustDocsServer::new(crate_name, documents, embeddings, startup_message)?;

    // Create the stdio transport
    let (stdin, stdout) = stdio();
    // Use the custom StdioTransport wrapper
    let transport = StdioTransport { reader: stdin, writer: stdout };

    println!("Rust Docs MCP server starting...");

    // Serve the server
    serve_server(service, transport).await?; // Use imported serve_server

    println!("Rust Docs MCP server stopped.");
    Ok(())
}
