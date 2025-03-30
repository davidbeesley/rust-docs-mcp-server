use scraper::{Html, Selector};
use std::{fs::{self, File, create_dir_all}, io::Write, path::PathBuf}; // Added PathBuf
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
    #[error("Failed to strip prefix '{prefix}' from path '{path}': {source}")] // Improved error
    StripPrefix { prefix: PathBuf, path: PathBuf, source: std::path::StripPrefixError },
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
pub fn load_documents(
    crate_name: &str,
    crate_version_req: &str,
    features: Option<&Vec<String>>, // Add optional features parameter
) -> Result<Vec<Document>, DocLoaderError> {
    eprintln!(
        "[DEBUG] load_documents called with crate_name: '{}', crate_version_req: '{}', features: {:?}",
        crate_name, crate_version_req, features
    );
    let mut documents = Vec::new();

    let temp_dir = tempdir().map_err(DocLoaderError::TempDirCreationFailed)?;
    let temp_dir_path = temp_dir.path();
    let temp_manifest_path = temp_dir_path.join("Cargo.toml");

    eprintln!(
        "Generating documentation for crate '{}' (Version Req: '{}', Features: {:?}) in temporary directory: {}",
        crate_name,
        crate_version_req,
        features, // Log features
        temp_dir_path.display()
    );

    // Create a temporary Cargo.toml using the version requirement and features
    let features_string = features
        .filter(|f| !f.is_empty()) // Only add features if provided and not empty
        .map(|f| {
            let feature_list = f.iter().map(|feat| format!("\"{}\"", feat)).collect::<Vec<_>>().join(", ");
            format!(", features = [{}]", feature_list)
        })
        .unwrap_or_default(); // Use empty string if no features

    let cargo_toml_content = format!(
        r#"[package]
name = "temp-doc-crate"
version = "0.1.0"
edition = "2021"

[lib] # Add an empty lib target to satisfy Cargo

[dependencies]
{} = {{ version = "{}"{} }}
"#,
        crate_name, crate_version_req, features_string // Use the version requirement string and features string here
    );

    // Create the src directory and an empty lib.rs file
    let src_path = temp_dir_path.join("src");
    create_dir_all(&src_path)?;
    File::create(src_path.join("lib.rs"))?;
    eprintln!("[DEBUG] Created empty src/lib.rs at: {}", src_path.join("lib.rs").display());

    let mut temp_manifest_file = File::create(&temp_manifest_path)?;
    temp_manifest_file.write_all(cargo_toml_content.as_bytes())?;
    eprintln!("[DEBUG] Created temporary manifest at: {}", temp_manifest_path.display());
    eprintln!("[DEBUG] Temporary Manifest Content:\n{}", cargo_toml_content); // Log content


    // --- Use Cargo API ---
    let mut config = GlobalContext::default()?; // Make mutable
    // Configure context (set quiet to false for more detailed errors)
    config.configure(
        0,     // verbose
        true, // quiet
        None,  // color
        false, // frozen
        false, // locked
        false, // offline
        &None, // target_dir (Using ws.set_target_dir instead)
        &[],   // unstable_flags
        &[],   // cli_config
    )?;
    // config.shell().set_verbosity(Verbosity::Quiet); // Keep commented

    // Use the temporary manifest path for the Workspace
    let mut ws = Workspace::new(&temp_manifest_path, &config)?; // Make ws mutable
    eprintln!("[DEBUG] Workspace target dir before set: {}", ws.target_dir().as_path_unlocked().display());
    // Set target_dir directly on Workspace
    ws.set_target_dir(cargo::util::Filesystem::new(temp_dir_path.to_path_buf()));
    eprintln!("[DEBUG] Workspace target dir after set: {}", ws.target_dir().as_path_unlocked().display());

    // Create CompileOptions, relying on ::new for BuildConfig
    let mut compile_opts = CompileOptions::new(&config, cargo::core::compiler::CompileMode::Doc { deps: false, json: false })?;
    // Specify the package explicitly
    let package_spec = crate_name.to_string(); // Just use name (with underscores)
    compile_opts.cli_features = CliFeatures::new_all(false); // Use new_all(false) - applies to the temp crate, not dependency
    compile_opts.spec = Packages::Packages(vec![package_spec.clone()]); // Clone spec

    // Create DocOptions: Pass compile options
    let doc_opts = DocOptions {
        compile_opts,
        open_result: false, // Don't open in browser
        output_format: ops::OutputFormat::Html,
    };
    eprintln!("[DEBUG] package_spec for CompileOptions: '{}'", package_spec);

    // Debug print relevant options before calling ops::doc
    eprintln!("[DEBUG] CompileOptions spec: {:?}", doc_opts.compile_opts.spec);
    eprintln!("[DEBUG] CompileOptions cli_features: {:?}", doc_opts.compile_opts.cli_features); // Features for temp crate
    eprintln!("[DEBUG] CompileOptions build_config mode: {:?}", doc_opts.compile_opts.build_config.mode);
    eprintln!("[DEBUG] DocOptions output_format: {:?}", doc_opts.output_format);

    ops::doc(&ws, &doc_opts).map_err(DocLoaderError::CargoLib)?; // Use ws
    // --- End Cargo API ---

    // --- Find the actual documentation directory ---
    // Iterate through subdirectories in `target/doc` and find the one containing `index.html`.
    let base_doc_path = temp_dir_path.join("doc");
    eprintln!("[DEBUG] Base doc path: {}", base_doc_path.display());

    let mut target_docs_path: Option<PathBuf> = None;
    let mut found_count = 0;

    if base_doc_path.is_dir() {
        for entry_result in fs::read_dir(&base_doc_path)? {
            let entry = entry_result?;
            eprintln!("[DEBUG] Checking directory entry: {}", entry.path().display()); // Log entry being checked
            if entry.file_type()?.is_dir() {
                let dir_path = entry.path();
                let index_html_path = dir_path.join("index.html");
                if index_html_path.is_file() {
                    eprintln!("[DEBUG] Found potential docs directory with index.html: {}", dir_path.display());
                    if target_docs_path.is_none() {
                        target_docs_path = Some(dir_path);
                    }
                    found_count += 1;
                } else {
                     eprintln!("[DEBUG] Skipping directory without index.html: {}", dir_path.display());
                }
            }
        }
    }

    let docs_path = match (found_count, target_docs_path) {
        (1, Some(path)) => {
            eprintln!("[DEBUG] Confirmed unique documentation directory: {}", path.display());
            path
        },
        (0, _) => {
            return Err(DocLoaderError::CargoLib(anyhow::anyhow!(
                "Could not find any subdirectory containing index.html within '{}'. Cargo doc might have failed or produced unexpected output.",
                base_doc_path.display()
            )));
        },
        (count, _) => {
             return Err(DocLoaderError::CargoLib(anyhow::anyhow!(
                "Expected exactly one subdirectory containing index.html within '{}', but found {}. Cannot determine the correct documentation path.",
                base_doc_path.display(), count
            )));
        }
    };
    // --- End finding documentation directory ---

    eprintln!("Using documentation path: {}", docs_path.display()); // Log the path we are actually using

    // Define the CSS selector for the main content area in rustdoc HTML
    // This might need adjustment based on the exact rustdoc version/theme
    let content_selector = Selector::parse("section#main-content.content")
        .map_err(|e| DocLoaderError::Selector(e.to_string()))?;
    eprintln!("[DEBUG] Calculated final docs_path: {}", docs_path.display());

    eprintln!("Starting document loading from: {}", docs_path.display());

    for entry in WalkDir::new(&docs_path)
        .into_iter()
        .filter_map(Result::ok) // Ignore errors during iteration for now
        .filter(|e| !e.file_type().is_dir() && e.path().extension().is_some_and(|ext| ext == "html"))
    {
        let path = entry.path();
        // Calculate path relative to the docs_path root
        let relative_path = path.strip_prefix(&docs_path).map_err(|e| {
            // Provide more context in the error message using the new error variant
            DocLoaderError::StripPrefix {
                prefix: docs_path.to_path_buf(),
                path: path.to_path_buf(),
                source: e,
            }
        })?;
        let path_str = relative_path.to_string_lossy().to_string(); // Use the relative path
        // eprintln!("Processing file: {} (relative: {})", path.display(), path_str); // Updated debug log

        // eprintln!("  Reading file content..."); // Added
        let html_content = fs::read_to_string(path)?; // Still read from the absolute path
        // eprintln!("  Parsing HTML..."); // Added

        // Parse the HTML document
        let document = Html::parse_document(&html_content);

        // Select the main content element
        if let Some(main_content_element) = document.select(&content_selector).next() {
            // Extract all text nodes within the main content
            // eprintln!("  Extracting text content..."); // Added
            let text_content: String = main_content_element
                .text()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<&str>>()
                .join("\n"); // Join text nodes with newlines

            if !text_content.is_empty() {
                // eprintln!("  Extracted content ({} chars)", text_content.len()); // Uncommented and simplified
                documents.push(Document {
                    path: path_str,
                    content: text_content,
                });
            } else {
                // eprintln!("No text content found in main section for: {}", path.display()); // Verbose logging
            }
        } else {
             // eprintln!("'main-content' selector not found for: {}", path.display()); // Verbose logging
             // Optionally handle files without the main content selector differently
        }
    }

    eprintln!("Finished document loading. Found {} documents.", documents.len());
    Ok(documents)
}