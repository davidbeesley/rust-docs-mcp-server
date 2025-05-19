use rustdocs_mcp_server::{
    doc_loader::{self, Document},
    document_chunker::{ChunkerConfig, DocumentChunker},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load documentation for a crate (e.g., serde)
    let crate_name = "serde";
    let crate_version = "1.0";
    let docs = doc_loader::load_documents(crate_name, crate_version, None)?;
    
    println!("Loaded {} documents for {}", docs.len(), crate_name);
    
    // Create a custom chunker configuration
    let config = ChunkerConfig {
        min_chunk_size: 512,     // 512 bytes minimum
        target_chunk_size: 1024, // 1KB target size
        max_chunk_size: 4096,    // 4KB maximum
        window_size: 16,         // 16-byte rolling window
        mask_bits: 10,           // 1/1024 chance of boundary detection
    };
    
    // Initialize the chunker with our configuration
    let chunker = DocumentChunker::new(config);
    
    // Chunk all documents
    let chunks = chunker.chunk_documents(&docs);
    
    println!("Generated {} chunks from {} documents", chunks.len(), docs.len());
    
    // Print a few example chunks
    println!("\nExample chunks:");
    for (i, chunk) in chunks.iter().take(3).enumerate() {
        println!("Chunk #{} (ID: {})", i + 1, &chunk.id[0..8]);
        println!("Source: {}", chunk.source_path);
        println!("Content: {} bytes", chunk.content.len());
        println!("Preview: {}", &chunk.content[0..chunk.content.len().min(100)].replace('\n', " "));
        println!("---");
    }
    
    // Demonstrate chunk stability
    println!("\nDemonstrating chunk stability:");
    
    // Create a slightly modified version of a document
    let original_doc = &docs[0];
    let modified_content = original_doc.content.replacen("rust", "Rust", 1);
    let modified_doc = Document {
        path: original_doc.path.clone(),
        content: modified_content,
    };
    
    // Chunk both versions
    let original_chunks = chunker.chunk_document(&original_doc.content, &original_doc.path);
    let modified_chunks = chunker.chunk_document(&modified_doc.content, &modified_doc.path);
    
    // Count matching chunks
    let mut matching_chunks = 0;
    let mut total_chunks = original_chunks.len().max(modified_chunks.len());
    
    for (i, original) in original_chunks.iter().enumerate() {
        if i < modified_chunks.len() {
            let modified = &modified_chunks[i];
            if original.id == modified.id {
                matching_chunks += 1;
            } else {
                println!("Chunk #{} changed. This is expected for the chunk containing the modification.", i + 1);
            }
        }
    }
    
    println!(
        "Chunk stability: {}/{} chunks preserved ({:.1}%) after minor modification",
        matching_chunks,
        total_chunks,
        (matching_chunks as f64 / total_chunks as f64) * 100.0
    );
    
    Ok(())
}