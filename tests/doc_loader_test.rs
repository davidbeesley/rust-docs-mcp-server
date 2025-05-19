use rustdocs_mcp_server::doc_loader;
use std::fs;
use std::path::Path;

#[test]
fn test_load_documents_from_cargo_doc() {
    // Skip if docs don't exist
    if !Path::new("./target/doc").exists() {
        println!("Skipping test_load_documents_from_cargo_doc as ./target/doc doesn't exist");
        return;
    }

    // Try to load the documents for this crate
    let result = doc_loader::load_documents_from_cargo_doc("rustdocs_mcp_server");

    // Check if we got documents or a proper error
    match result {
        Ok(docs) => {
            println!("Loaded {} documents for rustdocs_mcp_server", docs.len());
            assert!(!docs.is_empty(), "Should have loaded at least one document");
            
            // Check that document content is not empty
            for doc in docs {
                assert!(!doc.content.is_empty(), "Document content should not be empty");
                assert!(!doc.path.is_empty(), "Document path should not be empty");
            }
        }
        Err(e) => {
            println!("Error loading documents: {}", e);
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
fn test_process_html_documents() {
    // Create a temporary test directory
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let test_doc_path = temp_dir.path().join("test_crate");
    fs::create_dir_all(&test_doc_path).expect("Failed to create test doc directory");
    
    // Create a simple HTML file with rustdoc structure
    let html_content = r#"
    <!DOCTYPE html>
    <html>
    <head><title>Test Crate Documentation</title></head>
    <body>
        <section id="main-content" class="content">
            <p>This is a test document for the rustdoc format.</p>
            <p>It includes multiple paragraphs of content.</p>
            <pre>fn test_function() -> bool { true }</pre>
        </section>
    </body>
    </html>
    "#;
    
    fs::write(test_doc_path.join("index.html"), html_content)
        .expect("Failed to write test HTML file");
    
    // Try to load the documents from this directory
    let result = doc_loader::load_documents_from_cargo_doc("test_crate");
    
    // Should fail as this isn't in the expected cargo doc location
    assert!(result.is_err());
    
    // Clean up temp directory
    temp_dir.close().expect("Failed to clean up temp directory");
}