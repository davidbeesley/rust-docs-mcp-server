use ndarray::ArrayView1;
use rmcp::Service;
use rustdocs_mcp_server::{
    embeddings::{Embedding, EmbeddingProvider, OPENAI_CLIENT, cosine_similarity},
    server::RustDocsServer,
};
use std::env;

// Helper function to set up test environment
fn setup_env() {
    // Only set if not already set
    if env::var("OPENAI_API_KEY").is_err() {
        // Safe because we're in a controlled test environment
        unsafe {
            env::set_var("OPENAI_API_KEY", "dummy_key_for_tests");
        }
    }

    // Initialize the OpenAI client if not already set
    if OPENAI_CLIENT.get().is_none() {
        let client = async_openai::Client::new();
        let _ = OPENAI_CLIENT.set(client);
    }
}

#[test]
fn test_server_initialization() {
    setup_env();

    // Test creating a new server instance
    let result = RustDocsServer::new("Test startup message".to_string());
    assert!(result.is_ok());

    // We can't easily test the private fields directly, but we can verify the server was created
    let _server = result.unwrap();
}


#[tokio::test]
async fn test_similarity_matching() {
    setup_env();

    // Create test embeddings
    let doc1_embedding = Embedding::new(
        vec![0.5, 0.2, 0.1],
        EmbeddingProvider::OpenAI,
        "test-model".to_string(),
    );

    let doc2_embedding = Embedding::new(
        vec![0.1, 0.1, 0.9],
        EmbeddingProvider::OpenAI,
        "test-model".to_string(),
    );

    let query_embedding = Embedding::new(
        vec![0.1, 0.1, 0.8],
        EmbeddingProvider::OpenAI,
        "test-model".to_string(),
    );

    // Since find_best_match is private, we'll test the underlying cosine_similarity function
    // that it uses instead

    // Convert embeddings to arrays for cosine similarity calculation
    let doc1_array = doc1_embedding.to_array();
    let doc2_array = doc2_embedding.to_array();
    let query_array = query_embedding.to_array();

    // Calculate similarities
    let sim1 = cosine_similarity(query_array.view(), doc1_array.view());
    let sim2 = cosine_similarity(query_array.view(), doc2_array.view());

    // Doc2 should be more similar to the query
    assert!(sim2 > sim1);
    assert!(sim2 > 0.9); // Should be very high similarity

    // Also test with a simple manual calculation for a few values
    let test_v1 = ArrayView1::from(&[1.0, 0.0, 0.0]);
    let test_v2 = ArrayView1::from(&[0.0, 1.0, 0.0]);
    let test_v3 = ArrayView1::from(&[1.0, 1.0, 0.0]);

    // Orthogonal vectors should have zero similarity
    assert_eq!(cosine_similarity(test_v1, test_v2), 0.0);

    // Same vector should have similarity of 1.0
    assert_eq!(cosine_similarity(test_v1, test_v1), 1.0);

    // Check v1 and v3 (angle of 45 degrees)
    let expected_sim = 1.0 / f32::sqrt(2.0);
    let actual_sim = cosine_similarity(test_v1, test_v3);
    assert!((actual_sim - expected_sim).abs() < 1e-6);
}

#[test]
fn test_server_info() {
    setup_env();

    let server = RustDocsServer::new("Test server".to_string()).expect("Failed to create server");

    // Test the get_info method which is part of the ServerHandler trait
    let info = server.get_info();

    // Check if the server info has expected values
    assert_eq!(info.server_info.name, "rust-docs-mcp-server");
    assert!(info.instructions.is_some()); // Should have instructions

    // Verify protocol version and capabilities
    assert!(info.capabilities.tools.is_some());
    assert!(info.capabilities.logging.is_some());
}

