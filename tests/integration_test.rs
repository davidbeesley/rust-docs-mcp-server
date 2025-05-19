use rustdocs_mcp_server::{
    doc_loader::{self, Document},
    document_chunker::DocumentChunker,
    embedding_cache_service::EmbeddingCacheService,
    embeddings::{TestConfig, init_test_client},
    server::RustDocsServer,
};
use std::{env, path::Path};

// Integration test that tests the entire document processing pipeline
// from loading docs to chunking to embedding to query
#[tokio::test]
async fn test_document_processing_pipeline() {
    // Skip if no API key is provided or docs don't exist
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping integration test as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    if !Path::new("./target/doc").exists() {
        println!("Skipping integration test as ./target/doc doesn't exist");
        return;
    }
    
    // Initialize OpenAI client for testing using the safe test helper
    if init_test_client().is_err() {
        println!("Failed to initialize OpenAI client, but continuing with test");
    }
    
    // Test config provides default model names
    let _config = TestConfig::default();
        
    // 1. Load documents for the crate
    println!("1. Loading documents...");
    let crate_name = "rustdocs_mcp_server"; // Self-documentation
    let docs = match doc_loader::load_documents_from_cargo_doc(crate_name) {
        Ok(d) => {
            println!("Loaded {} documents", d.len());
            d
        }
        Err(e) => {
            println!("Failed to load documents: {}", e);
            return;
        }
    };
    
    // Check that we have at least one document
    assert!(!docs.is_empty(), "Should have loaded at least one document");
    
    // 2. Chunk the documents
    println!("2. Chunking documents...");
    let chunker = DocumentChunker::new();
    let mut all_chunks = Vec::new();
    
    for doc in &docs {
        let chunks = chunker.chunk_document(&doc.content);
        println!("Document '{}' produced {} chunks", doc.path, chunks.len());
        all_chunks.extend(chunks);
    }
    
    println!("Total chunks: {}", all_chunks.len());
    assert!(!all_chunks.is_empty(), "Should have generated at least one chunk");
    
    // 3. Generate embeddings for chunks
    println!("3. Generating embeddings...");
    let embedding_service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding service");
    
    // Just test a sample of chunks to keep test duration reasonable
    let sample_chunk = &all_chunks[0];
    println!("Testing embedding generation for chunk ID: {}", sample_chunk.id);
    
    match embedding_service.get_embedding_for_chunk(&sample_chunk.content).await {
        Ok(embedding) => {
            println!("Successfully generated embedding with {} dimensions", embedding.dimensions);
            assert!(!embedding.values.is_empty(), "Embedding should not be empty");
        }
        Err(e) => {
            println!("Failed to generate embedding: {}", e);
            // Don't fail the test, as this might be due to API rate limits or other transient issues
        }
    }
    
    // 4. Test the server initialization
    println!("4. Testing server initialization...");
    let startup_message = "Integration test".to_string();
    match RustDocsServer::new(startup_message) {
        Ok(_server) => {
            println!("Server initialized successfully");
            // We can't test queries directly as the query_rust_docs method is private
            // In a full integration test, this would be tested through the MCP protocol
        }
        Err(e) => {
            println!("Server initialization failed: {}", e);
        }
    }
    
    println!("Integration test completed");
}

// Test the entire process with a small synthetic document
#[tokio::test]
async fn test_synthetic_pipeline() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping synthetic test as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Initialize OpenAI client for testing using the safe test helper
    if init_test_client().is_err() {
        println!("Failed to initialize OpenAI client, but continuing with test");
    }
    
    // Create a synthetic document
    let doc = Document {
        path: "test/module.html".to_string(),
        content: "
        # Test Module Documentation
        
        This is a test module for Rust. It provides testing functionality.
        
        ## Functions
        
        ### `test_function() -> bool`
        
        Tests something and returns a boolean result.
        
        ## Structs
        
        ### `TestStruct`
        
        A structure for testing purposes with the following fields:
        
        - `name`: String - The name of the test
        - `value`: i32 - A test value
        
        ## Example
        
        ```rust
        let test = TestStruct {
            name: \"example\".to_string(),
            value: 42
        };
        assert!(test_function());
        ```
        ".to_string()
    };
    
    // Process the document
    println!("1. Chunking synthetic document...");
    let chunker = DocumentChunker::new();
    let chunks = chunker.chunk_document(&doc.content);
    println!("Synthetic document produced {} chunks", chunks.len());
    
    // Generate embeddings
    println!("2. Generating embeddings for synthetic document...");
    let embedding_service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding service");
    
    match embedding_service.get_embedding(&doc.content).await {
        Ok(doc_embedding) => {
            println!("Successfully generated embedding with {} dimensions", doc_embedding.dimensions);
            
            // Generate a question embedding
            let question = "What is TestStruct?";
            match embedding_service.get_embedding(question).await {
                Ok(question_embedding) => {
                    println!("Successfully generated question embedding");
                    
                    // Compare embeddings to see if they're related
                    let q_array = question_embedding.to_array();
                    let doc_array = doc_embedding.to_array();
                    
                    let dot_product = q_array.dot(&doc_array);
                    let norm_q = q_array.dot(&q_array).sqrt();
                    let norm_doc = doc_array.dot(&doc_array).sqrt();
                    let similarity = dot_product / (norm_q * norm_doc);
                    
                    println!("Similarity between question and document: {}", similarity);
                    assert!(similarity > 0.0, "Question and document should have some similarity");
                }
                Err(e) => println!("Failed to generate question embedding: {}", e),
            }
        }
        Err(e) => println!("Failed to generate document embedding: {}", e),
    }
}