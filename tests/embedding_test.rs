use rustdocs_mcp_server::embeddings::{Embedding, EmbeddingProvider, cosine_similarity, OPENAI_CLIENT};
use rustdocs_mcp_server::embedding_cache_service::EmbeddingCacheService;
use ndarray::Array1;
use std::env;

#[test]
fn test_embedding_struct() {
    // Create a test embedding
    let values = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let provider = EmbeddingProvider::OpenAI;
    let model = "test-model".to_string();
    
    let embedding = Embedding::new(values.clone(), provider, model.clone());
    
    // Check properties
    assert_eq!(embedding.values, values);
    assert_eq!(embedding.provider, provider);
    assert_eq!(embedding.model, model);
    assert_eq!(embedding.dimensions, 5);
    
    // Test array conversion
    let array = embedding.to_array();
    assert_eq!(array.len(), 5);
    assert_eq!(array[0], 0.1);
    assert_eq!(array[4], 0.5);
}

#[test]
fn test_cosine_similarity() {
    // Test identical vectors
    let v1 = Array1::from(vec![0.1, 0.2, 0.3, 0.4, 0.5]);
    let v2 = Array1::from(vec![0.1, 0.2, 0.3, 0.4, 0.5]);
    
    let similarity = cosine_similarity(v1.view(), v2.view());
    assert!((similarity - 1.0).abs() < 1e-6, "Identical vectors should have similarity 1.0");
    
    // Test orthogonal vectors
    let v3 = Array1::from(vec![1.0, 0.0, 0.0]);
    let v4 = Array1::from(vec![0.0, 1.0, 0.0]);
    
    let similarity = cosine_similarity(v3.view(), v4.view());
    assert!(similarity.abs() < 1e-6, "Orthogonal vectors should have similarity 0.0");
    
    // Test opposite vectors
    let v5 = Array1::from(vec![0.1, 0.2, 0.3]);
    let v6 = Array1::from(vec![-0.1, -0.2, -0.3]);
    
    let similarity = cosine_similarity(v5.view(), v6.view());
    assert!((similarity + 1.0).abs() < 1e-6, "Opposite vectors should have similarity -1.0");
    
    // Test vectors with some similarity
    let v7 = Array1::from(vec![0.1, 0.2, 0.3, 0.4]);
    let v8 = Array1::from(vec![0.2, 0.3, 0.4, 0.5]);
    
    let similarity = cosine_similarity(v7.view(), v8.view());
    assert!(similarity > 0.0 && similarity < 1.0, "Similar vectors should have similarity between 0 and 1");
}

// Integration tests that require an OpenAI API key will be skipped unless the key is provided
#[tokio::test]
async fn test_embedding_cache_service() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_embedding_cache_service as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Initialize OpenAI client
    let client = async_openai::Client::new();
    if OPENAI_CLIENT.set(client).is_err() {
        println!("Failed to initialize OpenAI client, but continuing with test");
    }
    
    // Default embedding model
    let _embedding_model = "text-embedding-3-small";
    
    // Create the embedding cache service
    let service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding cache service");
    
    // Test document embedding
    let test_doc = "This is a test document for embedding";
    let result = service.get_embedding(test_doc).await;
    
    match result {
        Ok(embedding) => {
            // Verify we got a valid embedding back
            assert!(!embedding.values.is_empty(), "Embedding should not be empty");
            assert_eq!(embedding.provider, EmbeddingProvider::OpenAI);
            assert!(embedding.model.contains("text-embedding"), "Model should contain 'text-embedding'");
            
            // Test repeated embedding to check caching
            let result2 = service.get_embedding(test_doc).await;
            assert!(result2.is_ok(), "Second embedding request should succeed");
            
            // Embeddings should be identical for the same text
            let embedding2 = result2.unwrap();
            assert_eq!(embedding.values, embedding2.values, "Cached embeddings should be identical");
        }
        Err(e) => {
            // If we got an API error, that might be expected with an invalid key
            println!("OpenAI embedding error (expected if using invalid API key): {}", e);
        }
    }
}

#[tokio::test]
async fn test_embedding_for_chunk() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_embedding_for_chunk as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Initialize OpenAI client
    let client = async_openai::Client::new();
    if OPENAI_CLIENT.set(client).is_err() {
        println!("Failed to initialize OpenAI client, but continuing with test");
    }
    
    // Create the embedding cache service
    let service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding cache service");
    
    // Test multiple chunks to ensure they get different embeddings
    let chunk1 = "This is the first test chunk with specific content about Rust programming.";
    let chunk2 = "This second chunk contains different information about database systems.";
    
    let result1 = service.get_embedding_for_chunk(chunk1).await;
    let result2 = service.get_embedding_for_chunk(chunk2).await;
    
    if let (Ok(embedding1), Ok(embedding2)) = (&result1, &result2) {
        // Different content should produce different embeddings
        assert_ne!(embedding1.values, embedding2.values, "Different content should have different embeddings");
        
        // Dimensions should be the same
        assert_eq!(embedding1.dimensions, embedding2.dimensions, "Embedding dimensions should be consistent");
    } else {
        println!("Skipping embedding comparison due to API error");
    }
}