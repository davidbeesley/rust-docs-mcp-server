use rustdocs_mcp_server::doc_loader::{self, DocLoaderError};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
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

// Create a test HTML file with invalid structure (missing the main content section)
fn create_invalid_html_file(dir_path: &Path, filename: &str) -> std::path::PathBuf {
    let file_path = dir_path.join(filename);
    let html_content = r#"<!DOCTYPE html>
        <html>
        <head><title>Test Crate Documentation</title></head>
        <body>
            <div>This does not have the right selector</div>
        </body>
        </html>
        "#;

    fs::write(&file_path, html_content).expect("Failed to write test HTML file");
    file_path
}

// Create a non-HTML file
fn create_non_html_file(dir_path: &Path, filename: &str, content: &str) -> std::path::PathBuf {
    let file_path = dir_path.join(filename);
    fs::write(&file_path, content).expect("Failed to write test file");
    file_path
}

// Create a binary/non-text file
fn create_binary_file(dir_path: &Path, filename: &str) -> std::path::PathBuf {
    let file_path = dir_path.join(filename);
    let binary_data: Vec<u8> = (0..=255).collect();
    fs::write(&file_path, &binary_data).expect("Failed to write binary file");
    file_path
}

// We can now directly use the public process_html_documents function
// No need for a workaround since it's been made public
use rustdocs_mcp_server::doc_loader::process_html_documents;

// Create a comprehensive test structure with various edge cases
fn setup_comprehensive_test_structure() -> tempfile::TempDir {
    let temp_dir = tempdir().expect("Failed to create temporary directory");

    // 1. Create regular documentation files
    create_test_html_file(
        temp_dir.path(),
        "index.html",
        "Main index page for test crate.",
    );
    create_test_html_file(
        temp_dir.path(),
        "struct.TestStruct.html",
        "Test struct documentation.",
    );
    create_test_html_file(
        temp_dir.path(),
        "fn.test_function.html",
        "Test function documentation.",
    );

    // 2. Create a nested module structure
    let module_dir = temp_dir.path().join("test_module");
    fs::create_dir_all(&module_dir).expect("Failed to create module directory");
    create_test_html_file(
        &module_dir,
        "index.html",
        "Module index documentation.",
    );
    create_test_html_file(
        &module_dir,
        "struct.ModuleStruct.html",
        "Module struct documentation.",
    );

    // 3. Create a src directory (which should be ignored)
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir).expect("Failed to create src directory");
    create_test_html_file(
        &src_dir,
        "lib.rs.html",
        "Source code that should be ignored.",
    );

    // 4. Create duplicate basename files in different locations
    create_test_html_file(
        &module_dir,
        "duplicate.html",
        "Small duplicate file.",
    );
    // Create a larger duplicate file (which should be preferred)
    let large_duplicate_path = temp_dir.path().join("duplicate.html");
    let mut large_file = File::create(&large_duplicate_path).expect("Failed to create large file");
    let large_content = format!(
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
        "Large duplicate file with much more content.".repeat(100) // Make it larger
    );
    large_file.write_all(large_content.as_bytes()).expect("Failed to write large duplicate file");
    
    // 5. Create an invalid HTML file
    create_invalid_html_file(temp_dir.path(), "invalid.html");
    
    // 6. Create a non-HTML file
    create_non_html_file(temp_dir.path(), "readme.md", "# Test Readme\nThis is not an HTML file.");
    
    // 7. Create a binary file with HTML extension (should be skipped due to read failure)
    create_binary_file(temp_dir.path(), "binary.html");
    
    // 8. Create a file with empty content in the main section
    let empty_content_path = temp_dir.path().join("empty_content.html");
    let empty_content = r#"<!DOCTYPE html>
        <html>
        <head><title>Test Crate Documentation</title></head>
        <body>
            <section id="main-content" class="content">
            </section>
        </body>
        </html>
        "#;
    fs::write(&empty_content_path, empty_content).expect("Failed to write empty content file");
    
    // Create a file we won't be able to access in Windows (but should fall back to error handling)
    if cfg!(target_os = "windows") {
        let forbidden_path = temp_dir.path().join("COM1.html"); // Reserved name in Windows
        let _ = fs::write(&forbidden_path, "<html></html>"); // Will likely fail but we don't care
    }
    
    temp_dir
}

#[test]
fn test_process_html_documents_direct() {
    // Now we can directly use the process_html_documents function
    let temp_dir = setup_comprehensive_test_structure();
    
    // Invoke the function directly
    let result = process_html_documents(temp_dir.path(), "test_crate");
    assert!(result.is_ok(), "Process HTML documents should succeed");
    
    let documents = result.unwrap();
    
    // Should have processed valid HTML files, excluding those in /src
    assert!(!documents.is_empty(), "Should have processed some documents");
    
    // The large duplicate file should be included
    let has_large_duplicate = documents.iter().any(|doc| {
        doc.path == "duplicate.html" && doc.content.contains("Large duplicate file")
    });
    assert!(has_large_duplicate, "Should include the larger duplicate file");
    
    // Files in src directory should be excluded
    let has_src_file = documents.iter().any(|doc| {
        doc.path.contains("src") && doc.path.contains("lib.rs.html")
    });
    assert!(!has_src_file, "Should not include files from the src directory");
    
    // Check that root index.html was included
    let has_root_index = documents.iter().any(|doc| {
        doc.path == "index.html" && doc.content.contains("Main index page")
    });
    assert!(has_root_index, "Should include the root index.html");
    
    // Files with no content in the main section should be excluded
    let has_empty_content = documents.iter().any(|doc| doc.path == "empty_content.html");
    assert!(!has_empty_content, "Should exclude files with empty content");
    
    // Invalid HTML files should be excluded
    let has_invalid_html = documents.iter().any(|doc| doc.path == "invalid.html");
    assert!(!has_invalid_html, "Should exclude invalid HTML files");
    
    // Non-HTML files should be excluded
    let has_non_html = documents.iter().any(|doc| doc.path == "readme.md");
    assert!(!has_non_html, "Should exclude non-HTML files");
}

#[test]
fn test_process_html_documents_empty_directory() {
    // Create empty directory
    let _temp_dir = tempdir().expect("Failed to create temporary directory");
    
    // We'll use the exported load_documents_from_cargo_doc function as a proxy
    // to test behavior with an empty directory
    
    // First move the empty directory to where load_documents_from_cargo_doc expects it
    let target_doc_path = Path::new("./target/doc/test_empty_crate");
    let _ = fs::create_dir_all(target_doc_path);
    let result = doc_loader::load_documents_from_cargo_doc("test_empty_crate");
    
    // We expect success but with an empty vector
    if result.is_ok() {
        let documents = result.unwrap();
        assert!(documents.is_empty(), "Empty directory should produce empty documents vector");
    } else {
        // If the directory doesn't exist, we might get a DocNotFound error, which is also acceptable
        match result.unwrap_err() {
            DocLoaderError::DocNotFound(_) => {
                // This is acceptable too, since the test might run where target/doc doesn't exist
            }
            other => panic!("Unexpected error: {:?}", other),
        }
    }
    
    // Clean up any directories we created
    let _ = fs::remove_dir_all(target_doc_path);
}

#[test]
fn test_process_html_documents_relative_paths() {
    // Test that relative paths are correctly calculated from the docs_path
    let temp_dir = tempdir().expect("Failed to create temporary directory");
    
    // Create a nested structure
    let nested_dir = temp_dir.path().join("nested").join("dir");
    fs::create_dir_all(&nested_dir).expect("Failed to create nested directory");
    
    create_test_html_file(
        &nested_dir,
        "deep_file.html",
        "Deep nested file content.",
    );
    
    // Now we can directly use the process_html_documents function
    
    // Process from the root temp directory
    let result = process_html_documents(temp_dir.path(), "test_crate");
    assert!(result.is_ok(), "Process HTML documents should succeed with nested paths");
    
    let documents = result.unwrap();
    assert!(!documents.is_empty(), "Should have processed the nested file");
    
    // Check if the relative path is correctly calculated
    let has_correct_path = documents.iter().any(|doc| {
        doc.path == PathBuf::from("nested/dir/deep_file.html").to_string_lossy()
    });
    
    assert!(has_correct_path, "Should calculate relative paths correctly");
}

#[test]
fn test_process_html_documents_html_parsing() {
    // Test the HTML parsing aspect specifically
    let temp_dir = tempdir().expect("Failed to create temporary directory");
    
    // Create HTML with complex structure
    let complex_html = r#"<!DOCTYPE html>
        <html>
        <head><title>Test Crate Documentation</title></head>
        <body>
            <section id="main-content" class="content">
                <h1>Title</h1>
                <p>Paragraph 1</p>
                <div>
                    <p>Nested paragraph</p>
                    <code>fn code_example() {}</code>
                </div>
                <pre>
                    let x = 5;
                    println!("{}", x);
                </pre>
            </section>
        </body>
        </html>
        "#;
    
    let complex_file_path = temp_dir.path().join("complex.html");
    fs::write(&complex_file_path, complex_html).expect("Failed to write complex HTML file");
    
    // Now we can directly use the process_html_documents function
    
    // Process the directory
    let result = process_html_documents(temp_dir.path(), "test_crate");
    assert!(result.is_ok(), "Process HTML documents should succeed with complex HTML");
    
    let documents = result.unwrap();
    assert!(!documents.is_empty(), "Should have processed the complex HTML file");
    
    // Check if all the content is extracted correctly
    let complex_doc = documents.iter().find(|doc| doc.path == "complex.html");
    assert!(complex_doc.is_some(), "Should find the complex HTML document");
    
    let content = &complex_doc.unwrap().content;
    assert!(content.contains("Title"), "Should extract heading");
    assert!(content.contains("Paragraph 1"), "Should extract paragraph");
    assert!(content.contains("Nested paragraph"), "Should extract nested paragraph");
    assert!(content.contains("fn code_example()"), "Should extract code");
    assert!(content.contains("let x = 5;"), "Should extract preformatted text");
}

#[test]
fn test_process_html_documents_invalid_selector() {
    // Test behavior when an invalid CSS selector is used
    // Since we can't modify the selector directly, this test is more theoretical
    let temp_dir = tempdir().expect("Failed to create temporary directory");
    
    // Create an HTML file with a slightly different selector
    let wrong_selector_html = r#"<!DOCTYPE html>
        <html>
        <head><title>Test Crate Documentation</title></head>
        <body>
            <section id="main_content" class="content">
                This has a slightly different selector (main_content vs main-content).
            </section>
        </body>
        </html>
        "#;
    
    let file_path = temp_dir.path().join("wrong_selector.html");
    fs::write(&file_path, wrong_selector_html).expect("Failed to write file");
    
    // Now we can directly use the process_html_documents function
    
    // The function should succeed but this document won't be included
    let result = process_html_documents(temp_dir.path(), "test_crate");
    assert!(result.is_ok(), "Should handle documents with no matching selector");
    
    let documents = result.unwrap();
    let has_wrong_selector = documents.iter().any(|doc| doc.path == "wrong_selector.html");
    assert!(!has_wrong_selector, "Should not include documents with no matching selector");
}