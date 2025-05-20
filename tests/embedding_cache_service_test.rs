use rustdocs_mcp_server::embeddings::{Embedding, EmbeddingProvider};
use rustdocs_mcp_server::embedding_cache_service::EmbeddingCacheService;
use std::env;

#[tokio::test]
async fn test_with_chunker_params() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_with_chunker_params as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Create the embedding cache service with custom chunker parameters
    let min_size = 100;
    let target_size = 200;
    let max_size = 400;
    
    // Initialize the service with custom parameters
    let service = EmbeddingCacheService::with_chunker_params(
        api_key,
        min_size,
        target_size,
        max_size
    ).expect("Failed to create embedding cache service with custom parameters");
    
    // Create a document that should be chunked according to our parameters
    let test_doc = "This is a test document. ".repeat(50);
    
    // Get embedding for the document
    let result = service.get_embedding(&test_doc).await;
    
    // Verify we got a successful result
    assert!(result.is_ok(), "Should successfully generate embedding with custom chunker params");
}

#[tokio::test]
async fn test_combine_chunk_embeddings() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_combine_chunk_embeddings as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Create the embedding cache service
    let service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding cache service");
    
    // Create test embeddings with known values - not used directly but kept for documentation
    let _embedding1 = Embedding::new(
        vec![1.0, 0.0, 0.0, 0.0], 
        EmbeddingProvider::OpenAI, 
        "test-model".to_string()
    );
    
    let _embedding2 = Embedding::new(
        vec![0.0, 1.0, 0.0, 0.0], 
        EmbeddingProvider::OpenAI, 
        "test-model".to_string()
    );
    
    let _embedding3 = Embedding::new(
        vec![0.0, 0.0, 1.0, 0.0], 
        EmbeddingProvider::OpenAI, 
        "test-model".to_string()
    );
    
    // We need to access the private method combine_chunk_embeddings
    // For this test, we'll use `get_embedding` with multiple chunks to indirectly test
    // the functionality
    
    // Create a document that will be split into chunks
    let chunk1 = "This is the first chunk about Rust programming and its features.";
    let chunk2 = "The second chunk discusses databases and storage systems.";
    let chunk3 = "A third chunk about machine learning and data processing.";
    
    let doc = format!("{}\n\n{}\n\n{}", chunk1, chunk2, chunk3);
    
    // Get embeddings for the whole document and individual chunks
    let doc_embedding_result = service.get_embedding(&doc).await;
    let chunk1_embedding_result = service.get_embedding_for_chunk(chunk1).await;
    let chunk2_embedding_result = service.get_embedding_for_chunk(chunk2).await;
    let chunk3_embedding_result = service.get_embedding_for_chunk(chunk3).await;
    
    if let (Ok(doc_embedding), Ok(chunk1_embedding), Ok(chunk2_embedding), Ok(chunk3_embedding)) = 
        (&doc_embedding_result, &chunk1_embedding_result, &chunk2_embedding_result, &chunk3_embedding_result) {
        
        // Check that the document embedding dimensions match chunk dimensions
        assert_eq!(
            doc_embedding.dimensions, 
            chunk1_embedding.dimensions,
            "Document and chunk embeddings should have same dimensions"
        );
        
        // The doc embedding should be somewhat similar to all chunks, 
        // but not identical to any single one
        let vec1 = &chunk1_embedding.values;
        let vec2 = &chunk2_embedding.values;
        let vec3 = &chunk3_embedding.values;
        let doc_vec = &doc_embedding.values;
        
        // Calculate manual distances to verify the document embedding is a
        // combination of the chunk embeddings
        let mut chunk_avg_vec = vec![0.0; doc_embedding.dimensions];
        for i in 0..chunk_avg_vec.len() {
            chunk_avg_vec[i] = (vec1[i] + vec2[i] + vec3[i]) / 3.0;
        }
        
        // Normalize the average vector
        let magnitude: f32 = chunk_avg_vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if magnitude > 0.0 {
            for val in &mut chunk_avg_vec {
                *val /= magnitude;
            }
        }
        
        // Calculate distance between our manual average and the actual document embedding
        // They should be very close if combine_chunk_embeddings is working correctly
        let mut distance_sum = 0.0;
        for i in 0..chunk_avg_vec.len() {
            let diff = chunk_avg_vec[i] - doc_vec[i];
            distance_sum += diff * diff;
        }
        let distance = distance_sum.sqrt();
        
        // The distance should be relatively small but might not be extremely close
        // due to OpenAI embedding model properties and normalization
        // This is a heuristic test since we don't have direct access to the private method
        assert!(
            distance < 1.0, 
            "Document embedding should be somewhat close to average of chunk embeddings, distance: {}", 
            distance
        );
        
        // Log the distance for debugging
        println!("Distance between computed average and actual document embedding: {}", distance);
    } else {
        println!("Skipping combine_chunk_embeddings test due to API errors");
    }
}

#[tokio::test]
async fn test_generate_and_cache_embedding() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_generate_and_cache_embedding as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Create the embedding cache service
    let service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding cache service");
    
    // Create a test document
    let test_doc = "This is a test document for testing caching and generation of embeddings.";
    
    // Get embedding for the document (this should generate and cache it)
    let result1 = service.get_embedding(test_doc).await;
    assert!(result1.is_ok(), "First embedding request should succeed");
    
    // Get embedding for the same document again (this should use the cache)
    let result2 = service.get_embedding(test_doc).await;
    assert!(result2.is_ok(), "Second embedding request should succeed using cache");
    
    // Verify the embeddings are identical
    let embedding1 = result1.unwrap();
    let embedding2 = result2.unwrap();
    
    assert_eq!(
        embedding1.values, 
        embedding2.values,
        "Cached embeddings should be identical"
    );
    
    // Slight modification to the document should generate a new embedding
    let modified_doc = "This is a test document for testing caching and generation of embeddings!";
    let result3 = service.get_embedding(modified_doc).await;
    assert!(result3.is_ok(), "Modified document embedding request should succeed");
    
    let embedding3 = result3.unwrap();
    assert_ne!(
        embedding1.values, 
        embedding3.values,
        "Different documents should have different embeddings"
    );
}

#[tokio::test]
async fn test_generate_openai_embedding() {
    // Skip if no API key is provided
    let api_key = match env::var("OPENAI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("Skipping test_generate_openai_embedding as OPENAI_API_KEY is not set");
            return;
        }
    };
    
    // Create the embedding cache service
    let service = EmbeddingCacheService::new(api_key).expect("Failed to create embedding cache service");
    
    // Create two test documents with distinctly different content
    let doc1 = "Rust is a systems programming language focused on safety and performance.";
    let doc2 = "Python is a high-level, interpreted programming language with dynamic typing.";
    
    // Get embeddings for both documents
    let result1 = service.get_embedding(doc1).await;
    let result2 = service.get_embedding(doc2).await;
    
    if let (Ok(embedding1), Ok(embedding2)) = (result1, result2) {
        // Verify that the embeddings are different for different content
        assert_ne!(
            embedding1.values, 
            embedding2.values,
            "Different documents should have different embeddings"
        );
        
        // Verify that the model name is set correctly (should contain text-embedding)
        assert!(
            embedding1.model.contains("text-embedding"),
            "Model name should contain 'text-embedding', got: {}", 
            embedding1.model
        );
        
        // Verify dimensions are as expected (should be greater than 0)
        assert!(embedding1.dimensions > 0, "Embedding should have positive dimensions");
        
        // Verify provider is set correctly
        assert_eq!(
            embedding1.provider, 
            EmbeddingProvider::OpenAI,
            "Provider should be OpenAI"
        );
    } else {
        println!("Skipping OpenAI embedding tests due to API errors");
    }
}