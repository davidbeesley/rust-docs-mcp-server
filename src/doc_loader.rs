use scraper::{Html, Selector};
use std::{collections::HashMap, fs::{self, File}, io::Write, path::{Path, PathBuf}};
use cargo::core::resolver::features::CliFeatures;

use cargo::core::Workspace;
use cargo::ops::{self, CompileOptions, DocOptions, Packages};
use cargo::util::context::GlobalContext;
use anyhow::Error as AnyhowError;
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
    #[error("Documentation not found: {0}")]
    DocNotFound(String),
    // Removed unused StripPrefix variant
}

// Simple struct to hold document content, maybe add path later if needed
#[derive(Debug, Clone)]
pub struct Document {
    pub path: String,
    pub content: String,
}

/// Processes HTML documents from a directory, extracting content from the main content area.
/// Used by both load_documents and load_documents_from_cargo_doc to avoid duplication.
fn process_html_documents(docs_path: &Path, crate_name: &str) -> Result<Vec<Document>, DocLoaderError> {
    let mut documents = Vec::new();
    
    // Define the CSS selector for the main content area in rustdoc HTML
    let content_selector = Selector::parse("section#main-content.content")
        .map_err(|e| DocLoaderError::Selector(e.to_string()))?;
    
    // Collect all HTML files
    let all_html_paths: Vec<PathBuf> = WalkDir::new(docs_path)
        .into_iter()
        .filter_map(Result::ok) // Ignore errors during iteration
        .filter(|e| {
            !e.file_type().is_dir() && e.path().extension().is_some_and(|ext| ext == "html")
        })
        .map(|e| e.into_path()) // Get the PathBuf
        .collect();
    
    eprintln!("[DEBUG] Found {} total HTML files for crate {}.", all_html_paths.len(), crate_name);
    
    // Group files by basename to handle duplicates
    let mut basename_groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for path in all_html_paths {
        if let Some(filename_osstr) = path.file_name() {
            if let Some(filename_str) = filename_osstr.to_str() {
                basename_groups
                    .entry(filename_str.to_string())
                    .or_default()
                    .push(path);
            } else {
                eprintln!("[WARN] Skipping file with non-UTF8 name: {}", path.display());
            }
        } else {
            eprintln!("[WARN] Skipping file with no name: {}", path.display());
        }
    }
    
    // Initialize paths to process
    let mut paths_to_process: Vec<PathBuf> = Vec::new();
    
    // Add the root index.html if it exists
    let root_index_path = docs_path.join("index.html");
    if root_index_path.is_file() {
        paths_to_process.push(root_index_path);
    }
    
    // Filter and add remaining files
    for (basename, mut paths) in basename_groups {
        // Skip index.html at this stage (except the root one added earlier)
        if basename == "index.html" {
            continue;
        }
        
        // Skip files within source code view directories
        if paths.first().is_some_and(|p| p.components().any(|comp| comp.as_os_str() == "src")) {
            continue;
        }
        
        if paths.len() == 1 {
            // Single file with this basename, keep it
            paths_to_process.push(paths.remove(0));
        } else {
            // Multiple files with the same basename, keep the largest one
            let largest_path_result: Result<Option<(PathBuf, u64)>, std::io::Error> = 
                paths.into_iter().try_fold(None::<(PathBuf, u64)>, |largest, current| {
                    let current_meta = fs::metadata(&current)?;
                    let current_size = current_meta.len();
                    match largest {
                        None => Ok(Some((current, current_size))),
                        Some((largest_path_so_far, largest_size_so_far)) => {
                            if current_size > largest_size_so_far {
                                Ok(Some((current, current_size)))
                            } else {
                                Ok(Some((largest_path_so_far, largest_size_so_far)))
                            }
                        }
                    }
                });
            
            match largest_path_result {
                Ok(Some((p, _size))) => {
                    paths_to_process.push(p);
                }
                Ok(None) => {
                    eprintln!("[WARN] No files found for basename '{}' during size comparison.", basename);
                }
                Err(e) => {
                    eprintln!("[WARN] Error getting metadata for basename '{}', skipping: {}", basename, e);
                }
            }
        }
    }
    
    eprintln!("[DEBUG] Filtered down to {} files to process for crate {}.", paths_to_process.len(), crate_name);
    
    // Process the filtered list of files
    for path in paths_to_process {
        // Calculate path relative to the docs_path
        let relative_path = match path.strip_prefix(docs_path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                eprintln!("[WARN] Failed to strip prefix {} from {}: {}", 
                    docs_path.display(), path.display(), e);
                continue;
            }
        };
        let path_str = relative_path.to_string_lossy().to_string();
        
        let html_content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("[WARN] Failed to read file {}: {}", path.display(), e);
                continue;
            }
        };
        
        let document = Html::parse_document(&html_content);
        
        if let Some(main_content_element) = document.select(&content_selector).next() {
            let text_content: String = main_content_element
                .text()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<&str>>()
                .join("\n");
            
            if !text_content.is_empty() {
                documents.push(Document {
                    path: path_str,
                    content: text_content,
                });
            }
        }
    }
    
    Ok(documents)
}


/// Generates documentation for a given crate in a temporary directory,
/// then loads and parses the HTML documents.
/// Extracts text content from the main content area of rustdoc generated HTML.
pub fn load_documents(
    crate_name: &str,
    crate_version_req: &str,
    features: Option<&Vec<String>>, // Add optional features parameter
) -> Result<Vec<Document>, DocLoaderError> {
    let temp_dir = tempdir().map_err(DocLoaderError::TempDirCreationFailed)?;
    let temp_dir_path = temp_dir.path();
    let temp_manifest_path = temp_dir_path.join("Cargo.toml");

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
    fs::create_dir_all(&src_path)?;
    File::create(src_path.join("lib.rs"))?;

    let mut temp_manifest_file = File::create(&temp_manifest_path)?;
    temp_manifest_file.write_all(cargo_toml_content.as_bytes())?;

    // --- Use Cargo API ---
    let mut config = GlobalContext::default()?; // Make mutable
    // Configure context (set quiet to false for more detailed errors)
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

    // Use the temporary manifest path for the Workspace
    let mut ws = Workspace::new(&temp_manifest_path, &config)?; // Make ws mutable
    // Set target_dir directly on Workspace
    ws.set_target_dir(cargo::util::Filesystem::new(temp_dir_path.to_path_buf()));

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

    ops::doc(&ws, &doc_opts).map_err(DocLoaderError::CargoLib)?; // Use ws
    // --- End Cargo API ---

    // --- Find the actual documentation directory ---
    // Iterate through subdirectories in `target/doc` and find the one containing `index.html`.
    let base_doc_path = temp_dir_path.join("doc");

    let mut target_docs_path: Option<PathBuf> = None;
    let mut found_count = 0;

    if base_doc_path.is_dir() {
        for entry_result in fs::read_dir(&base_doc_path)? {
            let entry = entry_result?;
            if entry.file_type()?.is_dir() {
                let dir_path = entry.path();
                let index_html_path = dir_path.join("index.html");
                if index_html_path.is_file() {
                    if target_docs_path.is_none() {
                        target_docs_path = Some(dir_path);
                    }
                    found_count += 1;
                }
            }
        }
    }

    let docs_path = match (found_count, target_docs_path) {
        (1, Some(path)) => {
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

    // Process the HTML documents using our helper function
    let documents = process_html_documents(&docs_path, crate_name)?;

    eprintln!("Finished document loading. Found {} final documents.", documents.len());
    Ok(documents)
}

/// Loads documentation for a crate from the local cargo doc output directory.
/// Extracts text content from the main content area of rustdoc generated HTML.
/// 
/// # Arguments
/// * `crate_name` - The name of the crate to load documentation for
/// 
/// # Returns
/// * `Result<Vec<Document>, DocLoaderError>` - A vector of documents with path and content
pub fn load_documents_from_cargo_doc(crate_name: &str) -> Result<Vec<Document>, DocLoaderError> {
    // Find the target directory in the current project
    // The standard location is `./target/doc/`
    let target_doc_path = Path::new("./target/doc");
    
    if !target_doc_path.exists() {
        return Err(DocLoaderError::DocNotFound(format!(
            "Documentation directory not found at {}. Run `cargo doc` first.",
            target_doc_path.display()
        )));
    }
    
    // Check if the crate documentation exists
    let crate_doc_path = target_doc_path.join(crate_name.replace('-', "_"));
    
    if !crate_doc_path.exists() || !crate_doc_path.is_dir() {
        return Err(DocLoaderError::DocNotFound(format!(
            "Documentation for crate '{}' not found at {}. Make sure the crate name is correct and run `cargo doc --package {}`.", 
            crate_name, 
            crate_doc_path.display(),
            crate_name
        )));
    }
    
    // Process the documents using the shared helper function
    let documents = process_html_documents(&crate_doc_path, crate_name)?;
    
    eprintln!("Finished loading documents from local cargo doc. Found {} final documents for crate {}.", 
        documents.len(), crate_name);
    
    Ok(documents)
}