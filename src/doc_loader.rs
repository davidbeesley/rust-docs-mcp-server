use scraper::{Html, Selector};
use std::fs;
use std::path::Path;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum DocLoaderError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WalkDir Error: {0}")]
    WalkDir(#[from] walkdir::Error),
    // #[error("Failed to parse HTML for file: {0}")] // Commented out unused variant
    // HtmlParsing(String), // Commented out unused variant
    #[error("CSS selector error: {0}")]
    Selector(String), // Using String as SelectorErrorKind is not easily clonable/convertible
}

// Simple struct to hold document content, maybe add path later if needed
#[derive(Debug, Clone)]
pub struct Document {
    pub path: String,
    pub content: String,
}

/// Loads and parses HTML documents from a given directory path.
/// Extracts text content from the main content area of rustdoc generated HTML.
pub fn load_documents(docs_path: &str) -> Result<Vec<Document>, DocLoaderError> {
    let mut documents = Vec::new();
    let docs_path = Path::new(docs_path);

    // Define the CSS selector for the main content area in rustdoc HTML
    // This might need adjustment based on the exact rustdoc version/theme
    let content_selector = Selector::parse("section#main-content.content")
        .map_err(|e| DocLoaderError::Selector(e.to_string()))?;

    println!("Starting document loading from: {}", docs_path.display());

    for entry in WalkDir::new(docs_path)
        .into_iter()
        .filter_map(Result::ok) // Ignore errors during iteration for now
        .filter(|e| !e.file_type().is_dir() && e.path().extension().map_or(false, |ext| ext == "html"))
    {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        // println!("Processing file: {}", path.display()); // Uncommented

        // println!("  Reading file content..."); // Added
        let html_content = fs::read_to_string(path)?;
        // println!("  Parsing HTML..."); // Added

        // Parse the HTML document
        let document = Html::parse_document(&html_content);

        // Select the main content element
        if let Some(main_content_element) = document.select(&content_selector).next() {
            // Extract all text nodes within the main content
            // println!("  Extracting text content..."); // Added
            let text_content: String = main_content_element
                .text()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<&str>>()
                .join("\n"); // Join text nodes with newlines

            if !text_content.is_empty() {
                // println!("  Extracted content ({} chars)", text_content.len()); // Uncommented and simplified
                documents.push(Document {
                    path: path_str,
                    content: text_content,
                });
            } else {
                // println!("No text content found in main section for: {}", path.display()); // Verbose logging
            }
        } else {
             // println!("'main-content' selector not found for: {}", path.display()); // Verbose logging
             // Optionally handle files without the main content selector differently
        }
    }

    println!("Finished document loading. Found {} documents.", documents.len());
    Ok(documents)
}