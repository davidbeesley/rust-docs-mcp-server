use rustdocs_mcp_server::document_chunker::DocumentChunker;
use std::collections::HashSet;
use std::io;
use std::time::Instant;

fn main() -> io::Result<()> {
    // Create a document chunker with default settings
    let chunker = DocumentChunker::new();

    // Always use our sample document
    let input = generate_sample_document();
    println!("Using sample document ({} bytes)", input.len());

    // Time the chunking operation
    let start = Instant::now();
    let chunks = chunker.chunk_document(&input);
    let elapsed = start.elapsed();

    // Report stats
    println!("\nContent-Defined Chunking Results:");
    println!("--------------------------------");
    println!("Chunks created: {}", chunks.len());
    println!(
        "Average chunk size: {:.2} bytes",
        input.len() as f64 / chunks.len() as f64
    );
    println!("Time to chunk: {:.2?}", elapsed);

    // Show size distribution
    let mut sizes: Vec<usize> = chunks.iter().map(|c| c.content.len()).collect();
    sizes.sort_unstable();

    if !sizes.is_empty() {
        println!("\nChunk size distribution:");
        println!("  Min: {} bytes", sizes[0]);
        println!("  25%: {} bytes", sizes[sizes.len() / 4]);
        println!("  Median: {} bytes", sizes[sizes.len() / 2]);
        println!("  75%: {} bytes", sizes[3 * sizes.len() / 4]);
        println!("  Max: {} bytes", sizes[sizes.len() - 1]);
    }

    // Test for collisions
    let unique_ids: HashSet<String> = chunks.iter().map(|c| c.id.clone()).collect();
    println!("\nUnique chunk IDs: {}/{}", unique_ids.len(), chunks.len());
    if unique_ids.len() != chunks.len() {
        println!("WARNING: Hash collisions detected!");
    }

    // Print first few chunks
    println!("\nFirst few chunks:");
    for (i, chunk) in chunks.iter().take(3).enumerate() {
        println!("Chunk #{} (id: {}...)", i + 1, &chunk.id[0..16]);
        println!("  Size: {} bytes", chunk.content.len());
        println!(
            "  Preview: {}",
            chunk
                .content
                .chars()
                .take(100)
                .collect::<String>()
                .replace('\n', " ")
        );
        println!();
    }

    // Modification stability test
    println!("\nTesting chunk stability with modifications:");
    test_stability(&chunker);

    Ok(())
}

/// Generate a sample Rust documentation for testing
fn generate_sample_document() -> String {
    r#"# Rust Documentation Example

This is a sample document that mimics Rust documentation. It contains different sections that should be chunked based on natural content boundaries.

## Module Structure

The `document_chunker` module provides a Content-Defined Chunking (CDC) implementation for Rust documentation.

### DocumentChunker

The main struct that handles the chunking process.

```rust
pub struct DocumentChunker {
    min_chunk_size: usize,
    target_chunk_size: usize,
    max_chunk_size: usize,
}
```

## Functions

### new()

Creates a new `DocumentChunker` with default parameters.

```rust
fn new() -> Self {
    Self {
        min_chunk_size: DEFAULT_MIN_CHUNK_SIZE,
        target_chunk_size: DEFAULT_TARGET_CHUNK_SIZE,
        max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
    }
}
```

### with_params()

Creates a new `DocumentChunker` with custom parameters.

```rust
fn with_params(min_size: usize, target_size: usize, max_size: usize) -> Self {
    Self {
        min_chunk_size: min_size,
        target_chunk_size: target_size,
        max_chunk_size: max_size,
    }
}
```

### chunk_document()

Splits a document into content-defined chunks.

```rust
fn chunk_document(&self, document: &str) -> Vec<Chunk> {
    // Implementation details...
}
```

## Advanced Usage

When working with larger documents, you might want to adjust the chunking parameters for optimal performance. The default parameters are:

- Minimum chunk size: 1KB
- Target chunk size: 4KB
- Maximum chunk size: 8KB

These values can be adjusted based on your specific needs, considering factors like document size, embedding model context limits, and cache efficiency.

### Example

```rust
let chunker = DocumentChunker::with_params(2000, 5000, 10000);
let chunks = chunker.chunk_document(large_document);
```

## Performance Considerations

The chunking algorithm uses a rolling hash function that runs in O(n) time, where n is the document length. This makes it efficient even for large documents.

The SHA-256 hash function used for chunk IDs provides strong collision resistance, ensuring that different chunks have different IDs with very high probability.

For even higher performance in critical applications, consider using the FNV hasher provided by the `fnv` crate, which offers faster hashing at the cost of slightly reduced collision resistance.

### Benchmarks

| Document Size | Chunks | Time to Process |
|---------------|--------|-----------------|
| 10KB          | 3      | 0.1ms           |
| 100KB         | 25     | 1.2ms           |
| 1MB           | 250    | 12ms            |
| 10MB          | 2500   | 120ms           |

## Integration with Embedding Systems

When working with embedding systems like OpenAI or local models, content-defined chunks provide several advantages:

1. Only changed chunks need to be re-embedded when documents are updated
2. Chunk boundaries occur at natural breaks in content
3. Stable chunk IDs allow for efficient caching and deduplication

For embedding generation, you can process each chunk individually:

```rust
for chunk in chunker.chunk_document(document) {
    let embedding = generate_embedding(chunk.content);
    cache.store(chunk.id, embedding);
}
```

## Limitations

- Very small documents may not benefit from chunking
- The current implementation works best with text documents
- Binary data may not produce optimal chunk boundaries

## Future Work

- Implement parallel chunking for very large documents
- Add support for custom boundary detection heuristics
- Explore alternative rolling hash algorithms for specific use cases
"#.to_string()
}

/// Test stability by modifying a document and seeing which chunks change
fn test_stability(chunker: &DocumentChunker) {
    // Original document
    let original = r#"# Rust Documentation
This is a sample of Rust documentation. 
It demonstrates how the content-defined chunking algorithm works.

## First Section
This section contains some Rust code:

```rust
fn hello_world() {
    println!("Hello, world!");
}
```

## Second Section
This is the second section with more content.
The chunker should find natural boundaries in the text.

## Third Section 
This is some more documentation text.
It should be chunked appropriately.
"#;

    // Modified document (small change in middle)
    let modified = r#"# Rust Documentation
This is a sample of Rust documentation. 
It demonstrates how the content-defined chunking algorithm works.

## First Section
This section contains some Rust code:

```rust
fn hello_world() {
    println!("Hello, modified world!"); // CHANGED THIS LINE
}
```

## Second Section
This is the second section with more content.
The chunker should find natural boundaries in the text.

## Third Section 
This is some more documentation text.
It should be chunked appropriately.
"#;

    // Chunk both documents
    let original_chunks = chunker.chunk_document(original);
    let modified_chunks = chunker.chunk_document(modified);

    // Compare results
    println!("Original chunks: {}", original_chunks.len());
    println!("Modified chunks: {}", modified_chunks.len());

    // Create sets of chunk IDs for comparison
    let original_ids: HashSet<String> = original_chunks.iter().map(|c| c.id.clone()).collect();
    let modified_ids: HashSet<String> = modified_chunks.iter().map(|c| c.id.clone()).collect();

    // Find differences
    let unchanged: HashSet<_> = original_ids.intersection(&modified_ids).collect();
    let changed_orig: HashSet<_> = original_ids.difference(&modified_ids).collect();
    let changed_mod: HashSet<_> = modified_ids.difference(&original_ids).collect();

    println!("Unchanged chunks: {}", unchanged.len());
    println!("Changed in original: {}", changed_orig.len());
    println!("Changed in modified: {}", changed_mod.len());

    // Calculate stability percentage
    let stability = unchanged.len() as f64 / original_chunks.len() as f64 * 100.0;
    println!("Stability: {:.1}% (higher is better)", stability);
}
