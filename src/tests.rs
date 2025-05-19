#[cfg(test)]
mod tests {
    use std::env;
    use tokio;
    use crate::doc_loader::load_documents_from_cargo_doc;
    use crate::embedding_cache_service::EmbeddingCacheService;
    
    // Helper function to set up test environment
    fn setup_env() {
        // Use unsafe block for setting environment variables
        unsafe {
            // Set dummy API key for tests
            env::set_var("OPENAI_API_KEY", "dummy_key_for_tests");
            
            // Set model names for consistent testing
            env::set_var("EMBEDDING_MODEL", "text-embedding-3-small");
            
            // Set LLM model
            env::set_var("LLM_MODEL", "gpt-4o-mini-2024-07-18");
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
            },
            Err(e) => {
                if e.to_string().contains("Documentation not found") {
                    println!("Documentation not found, which is expected if 'cargo doc' hasn't been run");
                } else {
                    panic!("Unexpected error: {}", e);
                }
            }
        }
    }
    
    #[tokio::test]
    async fn test_embedding_cache_service() {
        setup_env();
        
        // Create the embedding cache service
        let service = EmbeddingCacheService::new("dummy_key_for_tests".to_string());
        
        // Test document embedding - note this will mock the API call in a real implementation
        // In this test context, we just verify it doesn't panic
        let test_doc = "This is a test document for embedding";
        let result = service.get_embedding(test_doc).await;
        
        // Since we're using a dummy API key, we expect an error from the OpenAI API
        assert!(result.is_err(), "Expected error with dummy API key");
        
        // Verify it's the right kind of error (API error)
        let error = result.unwrap_err();
        let error_string = error.to_string();
        assert!(
            error_string.contains("OpenAI API error") || 
            error_string.contains("HTTP Request Error"), 
            "Expected OpenAI API error, got: {}", error_string
        );
    }
    
    // Add more tests as needed...
}

// Include the module in main.rs with:
// #[cfg(test)]
// mod tests;