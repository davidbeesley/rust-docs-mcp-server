#[cfg(test)]
mod module_tests {
    use crate::doc_loader::load_documents_from_cargo_doc;
    use crate::embedding_cache_service::EmbeddingCacheService;
    use crate::embeddings::init_test_client;
    use std::env;

    // Helper function to set up test environment
    fn setup_env() {
        // Set environment variables for tests
        unsafe {
            env::set_var("OPENAI_API_KEY", "dummy_key_for_tests");
        }
    }

    #[test]
    fn test_load_documents_from_cargo_doc() {
        // Skip if docs don't exist
        if !std::path::Path::new("./target/doc").exists() {
            println!("Skipping test_load_documents_from_cargo_doc as ./target/doc doesn't exist");
            return;
        }

        // Try to load the documents for this crate
        let result = load_documents_from_cargo_doc("rustdocs_mcp_server");

        // Check if we got documents or a proper error
        match result {
            Ok(docs) => {
                println!("Loaded {} documents for rustdocs_mcp_server", docs.len());
                assert!(!docs.is_empty(), "Should have loaded at least one document");
            }
            Err(e) => {
                if e.to_string().contains("Documentation not found") {
                    println!(
                        "Documentation not found, which is expected if 'cargo doc' hasn't been run"
                    );
                } else {
                    panic!("Unexpected error: {}", e);
                }
            }
        }
    }

    #[test]
    // This isn't an async test because we aren't actually testing the async calls
    fn test_embedding_cache_service() {
        setup_env();
        
        // Initialize the test client
        let _ = init_test_client();

        // Create the embedding cache service
        let _service = match EmbeddingCacheService::new("dummy_key_for_tests".to_string()) {
            Ok(s) => s,
            Err(e) => {
                // The service should initialize without errors
                panic!("Failed to create embedding cache service: {}", e);
            }
        };

        // We're only testing if the service initializes correctly
        // Since cache_dir is private, let's just make sure we can determine the cache path
        let home_dir = dirs::home_dir().expect("Could not find home directory");
        let expected_cache_dir = home_dir.join(".rust-doc-embedding-cache");
        assert!(expected_cache_dir.exists(), "Cache directory should exist at {}", expected_cache_dir.display());
    }

    // Add more tests as needed...
}