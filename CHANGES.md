# Changes in v1.4.0

## Major Enhancements

1. **Local Documentation Support**
   - Added ability to use existing cargo doc output instead of generating a temporary project
   - Created `load_documents_from_cargo_doc` function to load documentation from `./target/doc/`
   - More efficient than generating documentation on-the-fly

2. **Dynamic Crate Querying**
   - Added support for specifying a custom crate name in queries via the `crate_name` parameter
   - Server can now answer questions about any locally documented crate without restarting
   - Maintains backward compatibility with server's configured crate

3. **Content-Defined Chunking (CDC)**
   - Implemented CDC algorithm to create stable document chunks
   - Each chunk gets its own unique content-based hash ID
   - Only modified chunks need to be re-embedded when a document changes
   - Creates natural boundaries based on content patterns
   - Maintains chunk stability across minor document updates
   
4. **Embedding Cache Service**
   - Created a new `embedding_cache_service.rs` module for caching embeddings
   - Uses SHA-256 hashing of chunk content for consistent caching
   - Stores embeddings in `~/.rust-doc-embedding-cache/`
   - Lazy-loads embeddings only when needed
   - Combines chunk embeddings for complete document queries

## Implementation Details

- Introduced the `Embedding` struct with proper provider and model information
- Added error types for improved error handling
- Created test cases to verify functionality
- Maintained backward compatibility with existing implementation