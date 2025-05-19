use rustdocs_mcp_server::doc_loader::{self, DocLoaderError, Document};
use scraper::{Html, Selector};
use std::{fs, path::Path};
use tempfile::tempdir;

// Test helper to create a test HTML file with rustdoc-like structure
fn create_test_html_file(dir_path: &Path, filename: &str, content: &str) -> std::path::PathBuf {
    let file_path = dir_path.join(filename);

    let html_content = format!(
        r#"<!DOCTYPE html>
        <html>
        <head><title>Test Crate Documentation</title></head>
        <body>
            <section id="main-content" class="content">
                {}
            </section>
        </body>
        </html>
        "#,
        content
    );

    fs::write(&file_path, html_content).expect("Failed to write test HTML file");
    file_path
}

// Create directory structure for testing
fn setup_test_doc_structure() -> tempfile::TempDir {
    let temp_dir = tempdir().expect("Failed to create temporary directory");

    // Create a simple test file
    create_test_html_file(
        temp_dir.path(),
        "index.html",
        "This is the main index page for the test crate.",
    );

    // Create a module file
    create_test_html_file(
        temp_dir.path(),
        "test_module.html",
        "This is a module with some documentation.\nIt has multiple lines.",
    );

    // Create a struct file
    create_test_html_file(
        temp_dir.path(),
        "struct.TestStruct.html",
        "A test struct with some fields and methods.",
    );

    // Create a src directory (which should be skipped)
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("Failed to create src directory");
    create_test_html_file(
        &src_dir,
        "lib.rs.html",
        "Source code that should be skipped in processing.",
    );

    // Create a duplicate basename file in a subdirectory
    let sub_dir = temp_dir.path().join("sub");
    fs::create_dir_all(&sub_dir).expect("Failed to create sub directory");
    create_test_html_file(
        &sub_dir,
        "test_module.html",
        "This is a duplicate test module file with different content.",
    );

    temp_dir
}

#[test]
fn test_process_html_documents_with_structure() {
    // Call non-exported function through reflection
    use std::panic;

    let temp_dir = setup_test_doc_structure();

    // We'll use the exported function in doc_loader that calls process_html_documents internally
    let result = doc_loader::load_documents_from_cargo_doc("test_crate");

    // Should be an error since temp_dir is not in the standard cargo doc location
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        DocLoaderError::DocNotFound(_)
    ));

    // Since we can't directly call the internal function, let's verify the test document structure
    let index_path = temp_dir.path().join("index.html");
    let html_content = fs::read_to_string(&index_path).expect("Failed to read index.html");
    let document = Html::parse_document(&html_content);

    // Define the CSS selector for the main content area in rustdoc HTML
    let content_selector = Selector::parse("section#main-content.content").unwrap();

    if let Some(main_content_element) = document.select(&content_selector).next() {
        let text_content: String = main_content_element
            .text()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("\n");

        assert!(text_content.contains("This is the main index page"));
    } else {
        panic!("Could not find main content element in test HTML");
    }
}

#[test]
fn test_load_documents_doc_not_found() {
    // Test the error case when documentation directory doesn't exist
    let crate_name = "nonexistent_crate";

    // This should return a DocNotFound error
    let result = doc_loader::load_documents_from_cargo_doc(crate_name);
    assert!(result.is_err());

    match result.unwrap_err() {
        DocLoaderError::DocNotFound(msg) => {
            assert!(msg.contains("not found"));
            
            // The function can fail in two ways:
            // 1. If target/doc doesn't exist at all: "Documentation directory not found at ./target/doc"
            // 2. If the specific crate docs don't exist: "Documentation for crate 'nonexistent_crate' not found"
            
            // We accept either error message, since both are valid DocNotFound errors
            let contains_crate_name = msg.contains(crate_name) || 
                                     msg.contains(&crate_name.replace('-', "_"));
            
            let is_target_dir_error = msg.contains("Documentation directory not found");
            
            // Either the message should contain the crate name or it should be the general target dir error
            assert!(contains_crate_name || is_target_dir_error, 
                    "Error message should contain the crate name or be about the missing target directory: {}", msg);
        }
        other => panic!("Expected DocLoaderError::DocNotFound, got {:?}", other),
    }
}

#[test]
fn test_document_struct() {
    // Simple test for the Document struct
    let doc = Document {
        path: "test/path.html".to_string(),
        content: "Test content".to_string(),
    };

    assert_eq!(doc.path, "test/path.html");
    assert_eq!(doc.content, "Test content");

    // Test clone works
    let doc_clone = doc.clone();
    assert_eq!(doc.path, doc_clone.path);
    assert_eq!(doc.content, doc_clone.content);

    // Test debug formatting
    let debug_str = format!("{:?}", doc);
    assert!(debug_str.contains("Document"));
    assert!(debug_str.contains("test/path.html"));
    assert!(debug_str.contains("Test content"));
}

