use scraper::{Html, Selector};
use std::fs;
use cargo::core::resolver::features::CliFeatures;
// use cargo::core::SourceId; // Removed unused import
// use cargo::util::Filesystem; // Removed unused import

use cargo::core::Workspace;
use cargo::ops::{self, CompileOptions, DocOptions, Packages};
use cargo::util::context::GlobalContext;
use anyhow::Error as AnyhowError;
// use std::process::Command; // Remove Command again
use tempfile::tempdir;
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum DocLoaderError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("WalkDir Error: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("CSS selector error: {0}")]
    Selector(String),
    #[error("Temporary directory creation failed: {0}")]
    TempDirCreationFailed(std::io::Error),
    #[error("Cargo library error: {0}")]
    CargoLib(#[from] AnyhowError), // Re-add CargoLib variant
}

// Simple struct to hold document content, maybe add path later if needed
#[derive(Debug, Clone)]
pub struct Document {
    pub path: String,
    pub content: String,
}

/// Generates documentation for a given crate in a temporary directory,
/// then loads and parses the HTML documents.
/// Extracts text content from the main content area of rustdoc generated HTML.
pub fn load_documents(crate_name: &str, _crate_version: &str) -> Result<Vec<Document>, DocLoaderError> { // Mark version as unused for now
    println!("[DEBUG] load_documents called with crate_name: '{}', crate_version: '{}'", crate_name, _crate_version);
    let mut documents = Vec::new();

    let temp_dir = tempdir().map_err(DocLoaderError::TempDirCreationFailed)?;
    let temp_dir_path = temp_dir.path();

    println!(
        "Generating documentation for crate '{}' in temporary directory: {}",
        crate_name,
        temp_dir_path.display()
    );

    // Execute `cargo doc` using std::process::Command
    // --- Use Cargo API ---
    let mut config = GlobalContext::default()?; // Make mutable
    // Configure context for quiet operation
    config.configure(
        0,     // verbose
        true,  // quiet
        None,  // color
        false, // frozen
        false, // locked
        false, // offline
        &None, // target_dir (Using ws.set_target_dir instead)
        &[],   // unstable_flags
        &[],   // cli_config
    )?;
    // config.shell().set_verbosity(Verbosity::Quiet); // Keep commented

    let current_dir = std::env::current_dir()?;
    let mut ws = Workspace::new(&current_dir.join("Cargo.toml"), &config)?; // Make ws mutable
    println!("[DEBUG] Workspace target dir before set: {}", ws.target_dir().as_path_unlocked().display());
    // Set target_dir directly on Workspace
    ws.set_target_dir(cargo::util::Filesystem::new(temp_dir_path.to_path_buf()));
    println!("[DEBUG] Workspace target dir after set: {}", ws.target_dir().as_path_unlocked().display());

    // Create CompileOptions, relying on ::new for BuildConfig
    let mut compile_opts = CompileOptions::new(&config, cargo::core::compiler::CompileMode::Doc { deps: false, json: false })?;
    // Specify the package explicitly
    let package_spec = crate_name.replace('-', "_"); // Just use name (with underscores)
    compile_opts.cli_features = CliFeatures::new_all(false); // Use new_all(false)
    compile_opts.spec = Packages::Packages(vec![package_spec.clone()]); // Clone spec

    // Create DocOptions: Pass compile options
    let doc_opts = DocOptions {
        compile_opts,
        open_result: false, // Don't open in browser
        output_format: ops::OutputFormat::Html,
    };
    println!("[DEBUG] package_spec for CompileOptions: '{}'", package_spec);

    ops::doc(&ws, &doc_opts).map_err(DocLoaderError::CargoLib)?; // Use ws
    // --- End Cargo API ---
    // Construct the path to the generated documentation within the temp directory
    // Cargo uses underscores in the directory path if the crate name has hyphens
    let crate_name_underscores = crate_name.replace('-', "_");
    let docs_path = temp_dir_path.join("doc").join(&crate_name_underscores);

    // Debug print relevant options before calling ops::doc
    println!("[DEBUG] CompileOptions spec: {:?}", doc_opts.compile_opts.spec);
    println!("[DEBUG] CompileOptions cli_features: {:?}", doc_opts.compile_opts.cli_features);
    println!("[DEBUG] CompileOptions build_config mode: {:?}", doc_opts.compile_opts.build_config.mode);
    println!("[DEBUG] DocOptions output_format: {:?}", doc_opts.output_format);
    if !docs_path.exists() || !docs_path.is_dir() {
         return Err(DocLoaderError::CargoLib(anyhow::anyhow!(
             "Generated documentation not found at expected path: {}. Check crate name and cargo doc output.",
             docs_path.display()
         )));
    }
    println!("Generated documentation path: {}", docs_path.display());

    println!("[DEBUG] ops::doc called successfully.");

    // Define the CSS selector for the main content area in rustdoc HTML
    // This might need adjustment based on the exact rustdoc version/theme
    let content_selector = Selector::parse("section#main-content.content")
        .map_err(|e| DocLoaderError::Selector(e.to_string()))?;
    println!("[DEBUG] Calculated final docs_path: {}", docs_path.display());

    println!("Starting document loading from: {}", docs_path.display());
        println!("[DEBUG] docs_path does not exist or is not a directory.");

    for entry in WalkDir::new(docs_path)
        .into_iter()
        .filter_map(Result::ok) // Ignore errors during iteration for now
        .filter(|e| !e.file_type().is_dir() && e.path().extension().is_some_and(|ext| ext == "html"))
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