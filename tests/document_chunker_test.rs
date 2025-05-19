use rustdocs_mcp_server::document_chunker::DocumentChunker;

#[test]
fn test_document_chunker_basic() {
    // Create a chunker with default parameters
    let chunker = DocumentChunker::new();
    
    // Test with small document (smaller than min chunk size)
    let small_doc = "This is a small test document.";
    let chunks = chunker.chunk_document(small_doc);
    
    // Should produce exactly one chunk
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, small_doc);
    
    // Verify chunk ID is deterministic
    let id1 = chunks[0].id.clone();
    let chunks_again = chunker.chunk_document(small_doc);
    assert_eq!(chunks_again[0].id, id1, "Chunk IDs should be deterministic");
    
    // Test with slightly modified content
    let modified_doc = "This is a small test Document."; // 'document' -> 'Document'
    let modified_chunks = chunker.chunk_document(modified_doc);
    assert_ne!(modified_chunks[0].id, id1, "Different content should have different IDs");
}

#[test]
fn test_document_chunker_large() {
    // Create a chunker with custom smaller parameters for testing
    let chunker = DocumentChunker::with_params(50, 100, 200);
    
    // Generate a larger document that should be split into multiple chunks
    let large_doc = (0..10).map(|_| "This is a paragraph that should contribute to the overall size of the document and force the chunker to create multiple chunks based on the content. ".repeat(5)).collect::<Vec<String>>().join("\n\n");
    
    // Should be well over the max chunk size
    assert!(large_doc.len() > 1000);
    
    // Chunk the document
    let chunks = chunker.chunk_document(&large_doc);
    
    // Should produce multiple chunks
    assert!(chunks.len() > 1, "Should create multiple chunks for a large document");
    
    // Check that no chunk exceeds the maximum size
    for chunk in &chunks {
        assert!(chunk.content.len() <= 200, "No chunk should exceed the maximum size");
    }
    
    // Check that most chunks are around the target size (except potentially the last one)
    for chunk in chunks.iter().take(chunks.len()-1) {
        assert!(chunk.content.len() >= 50, "Chunks should meet minimum size");
    }
    
    // Reassemble the document and check for content loss
    let reassembled = chunks.iter().map(|c| c.content.clone()).collect::<Vec<String>>().join("");
    assert_eq!(reassembled, large_doc, "Reassembled chunks should match the original document");
}

#[test]
fn test_chunk_stability() {
    // Test that changes to one part of a document only affect nearby chunks
    let chunker = DocumentChunker::with_params(100, 200, 300);
    
    // Create a document with distinct sections
    let part1 = "Section 1: This is the first section of the document. It contains unique content that should form its own chunk or chunks.".repeat(3);
    let part2 = "Section 2: This is the section we will modify. It contains content that will be changed in the test to see how it affects chunking.".repeat(3);
    let part3 = "Section 3: This is the final section of the document. It should remain in the same chunks even when section 2 is modified.".repeat(3);
    
    let original_doc = format!("{}\n\n{}\n\n{}", part1, part2, part3);
    let modified_doc = format!("{}\n\nMODIFIED: {}\n\n{}", part1, part2, part3);
    
    // Chunk both documents
    let original_chunks = chunker.chunk_document(&original_doc);
    let modified_chunks = chunker.chunk_document(&modified_doc);
    
    // Compare chunks
    let original_ids: Vec<String> = original_chunks.iter().map(|c| c.id.clone()).collect();
    let modified_ids: Vec<String> = modified_chunks.iter().map(|c| c.id.clone()).collect();
    
    // At least some chunks should be different
    assert_ne!(original_ids, modified_ids, "Changing content should result in different chunks");
    
    // But some chunks should stay the same (content-defined chunking preserves unmodified regions)
    let mut matching_chunks = 0;
    for id in &original_ids {
        if modified_ids.contains(id) {
            matching_chunks += 1;
        }
    }
    
    // We should have at least one matching chunk
    assert!(matching_chunks > 0, "Some chunks should remain unchanged");
}

#[test]
fn test_generate_chunk_id() {
    let chunker = DocumentChunker::new();
    
    // Test that identical content produces identical IDs
    let content = "This is a test for chunk ID generation";
    let id1 = chunker.generate_chunk_id(content);
    let id2 = chunker.generate_chunk_id(content);
    
    assert_eq!(id1, id2, "Identical content should produce identical chunk IDs");
    
    // Test that different content produces different IDs
    let different_content = "This is a test for chunk ID generation!"; // Added !
    let id3 = chunker.generate_chunk_id(different_content);
    
    assert_ne!(id1, id3, "Different content should produce different chunk IDs");
}