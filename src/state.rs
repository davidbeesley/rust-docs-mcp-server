use crate::doc_loader::Document; // Import Document from sibling module
use ndarray::Array1;
use rmcp::model::Tool; // Import Tool

// Placeholder for server state
#[derive(Debug)] // Added Debug derive
// Removed duplicate derive and misplaced use statement
pub struct ServerState {
    pub crate_name: String,
    // pub docs_path: String, // Commented out unused field
    pub documents: Vec<Document>,                 // Store loaded documents
    pub embeddings: Vec<(String, Array1<f32>)>, // Store path and embedding vector
    pub tools: Vec<Tool>,                       // Store tool definitions here
                                            // TODO: Add LlamaIndex equivalent state (e.g., index, query engine)
}