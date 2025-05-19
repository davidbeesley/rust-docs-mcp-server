use rustdocs_mcp_server::embeddings::{TestConfig, init_test_client};
use std::{env, path::Path};

/// Initializes the test environment for tests that need an OpenAI client.
/// Returns true if the environment was successfully set up, false otherwise.
pub fn setup_openai_env() -> bool {
    // Skip if no API key is provided
    if env::var("OPENAI_API_KEY").is_err() {
        eprintln!("OPENAI_API_KEY not set, skipping test");
        return false;
    }

    // Initialize OpenAI client using the safe test helper
    if init_test_client().is_err() {
        eprintln!("Failed to initialize OpenAI client");
        return false;
    }
    
    // Test config is used by tests to get model names without env vars
    let _config = TestConfig::default();
    
    true
}

/// Checks if the documentation directory exists.
/// Returns true if it exists, false otherwise.
pub fn check_doc_dir_exists() -> bool {
    if !Path::new("./target/doc").exists() {
        eprintln!("Documentation directory not found at ./target/doc");
        return false;
    }
    true
}

/// Creates a temporary HTML file with rustdoc structure for testing.
/// Returns the path to the created file and a cleanup function.
pub fn create_test_html_file() -> (tempfile::TempDir, std::path::PathBuf) {
    // Create a temporary directory
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let test_file_path = temp_dir.path().join("test.html");
    
    // Create a test HTML file with rustdoc structure
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
    
    std::fs::write(&test_file_path, html_content)
        .expect("Failed to write test HTML file");
    
    (temp_dir, test_file_path)
}

/// Creates a test document with specified content sections.
/// Returns a Document struct.
pub fn create_test_document(
    path: &str,
    title: &str,
    description: &str,
    code_examples: &[&str],
) -> rustdocs_mcp_server::Document {
    let mut content = format!("# {}\n\n{}\n\n", title, description);
    
    if !code_examples.is_empty() {
        content.push_str("## Examples\n\n");
        for example in code_examples {
            content.push_str(&format!("```rust\n{}\n```\n\n", example));
        }
    }
    
    rustdocs_mcp_server::Document {
        path: path.to_string(),
        content,
    }
}

/// Generates a long test document for chunking tests.
/// Returns the document content as a string.
pub fn generate_long_test_document(paragraphs: usize, paragraph_size: usize) -> String {
    let paragraph_template = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ";
    let mut content = String::new();
    
    for i in 0..paragraphs {
        content.push_str(&format!("## Section {}\n\n", i + 1));
        
        for _ in 0..paragraph_size {
            content.push_str(paragraph_template);
        }
        
        content.push_str("\n\n");
    }
    
    content
}