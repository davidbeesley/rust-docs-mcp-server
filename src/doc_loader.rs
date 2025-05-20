use scraper::{Html, Selector};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Error as AnyhowError;
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
    #[error("Cargo library error: {0}")]
    CargoLib(#[from] AnyhowError),
    #[error("Documentation not found: {0}")]
    DocNotFound(String),
}

// Simple struct to hold document content, maybe add path later if needed
#[derive(Debug, Clone)]
pub struct Document {
    pub path: String,
    pub content: String,
}

/// Processes HTML documents from a directory, extracting content from the main content area.
/// Used by both load_documents and load_documents_from_cargo_doc to avoid duplication.
pub fn process_html_documents(
    docs_path: &Path,
    crate_name: &str,
) -> Result<Vec<Document>, DocLoaderError> {
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

    eprintln!(
        "[DEBUG] Found {} total HTML files for crate {}.",
        all_html_paths.len(),
        crate_name
    );

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
                eprintln!(
                    "[WARN] Skipping file with non-UTF8 name: {}",
                    path.display()
                );
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
        if paths
            .first()
            .is_some_and(|p| p.components().any(|comp| comp.as_os_str() == "src"))
        {
            continue;
        }

        if paths.len() == 1 {
            // Single file with this basename, keep it
            paths_to_process.push(paths.remove(0));
        } else {
            // Multiple files with the same basename, keep the largest one
            let largest_path_result: Result<Option<(PathBuf, u64)>, std::io::Error> = paths
                .into_iter()
                .try_fold(None::<(PathBuf, u64)>, |largest, current| {
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
                    eprintln!(
                        "[WARN] No files found for basename '{}' during size comparison.",
                        basename
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[WARN] Error getting metadata for basename '{}', skipping: {}",
                        basename, e
                    );
                }
            }
        }
    }

    eprintln!(
        "[DEBUG] Filtered down to {} files to process for crate {}.",
        paths_to_process.len(),
        crate_name
    );

    // Process the filtered list of files
    for path in paths_to_process {
        // Calculate path relative to the docs_path
        let relative_path = match path.strip_prefix(docs_path) {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                eprintln!(
                    "[WARN] Failed to strip prefix {} from {}: {}",
                    docs_path.display(),
                    path.display(),
                    e
                );
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

    eprintln!(
        "Finished loading documents from local cargo doc. Found {} final documents for crate {}.",
        documents.len(),
        crate_name
    );

    Ok(documents)
}
