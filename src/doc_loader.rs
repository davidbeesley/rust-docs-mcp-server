use scraper::{Html, Selector};
use std::{collections::HashMap, fs, path::{Path, PathBuf}};
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
    #[error("Path prefix error: {0}")]
    StripPrefix(#[from] std::path::StripPrefixError),
    #[error("Documentation not found: {0}")]
    DocNotFound(String),
}

// Document struct to hold content and path
#[derive(Debug, Clone)]
pub struct Document {
    pub path: String,
    pub content: String,
}

/// Loads documentation from local target/doc directory for a given crate.
/// Extracts text content from the main content area of rustdoc generated HTML.
pub fn load_documents(
    workspace_path: &Path,
    crate_name: &str,
) -> Result<Vec<Document>, DocLoaderError> {
    let mut documents = Vec::new();
    
    // Look for documentation in the target/doc directory
    let target_doc_path = workspace_path.join("target").join("doc");
    
    // Find the specific crate documentation directory
    let crate_doc_path = find_crate_doc_directory(&target_doc_path, crate_name)?;
    
    eprintln!("Using documentation path: {}", crate_doc_path.display());

    // Define the CSS selector for the main content area in rustdoc HTML
    let content_selector = Selector::parse("section#main-content.content")
        .map_err(|e| DocLoaderError::Selector(e.to_string()))?;

    // --- Collect all HTML file paths first ---
    let all_html_paths: Vec<PathBuf> = WalkDir::new(&crate_doc_path)
        .into_iter()
        .filter_map(Result::ok) // Ignore errors during iteration
        .filter(|e| {
            !e.file_type().is_dir() && e.path().extension().is_some_and(|ext| ext == "html")
        })
        .map(|e| e.into_path()) // Get the PathBuf
        .collect();

    eprintln!("[DEBUG] Found {} total HTML files initially.", all_html_paths.len());

    // --- Group files by basename ---
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

    // --- Initialize paths_to_process and explicitly add the root index.html if it exists --- 
    let mut paths_to_process: Vec<PathBuf> = Vec::new();
    let root_index_path = crate_doc_path.join("index.html");
    if root_index_path.is_file() {
        paths_to_process.push(root_index_path);
    }

    // --- Filter based on duplicates and size ---
    for (basename, mut paths) in basename_groups {
        // Always ignore index.html at this stage (except the root one added earlier)
        if basename == "index.html" {
            continue;
        }

        // Also ignore files within source code view directories
        if paths.first().is_some_and(|p| p.components().any(|comp| comp.as_os_str() == "src")) {
            continue;
        }

        if paths.len() == 1 {
            // Single file with this basename (and not index.html), keep it
            paths_to_process.push(paths.remove(0));
        } else {
            // Multiple files with the same basename (duplicates)
            // Find the largest one by file size
            let largest_path_result: Result<Option<(PathBuf, u64)>, std::io::Error> = paths.into_iter().try_fold(None::<(PathBuf, u64)>, |largest, current| {
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

    eprintln!("[DEBUG] Filtered down to {} files to process.", paths_to_process.len());

    // --- Process the filtered list of files ---
    for path in paths_to_process {
        // Calculate path relative to the crate_doc_path root
        let relative_path = path.strip_prefix(&crate_doc_path)?.to_path_buf();
        let path_str = relative_path.to_string_lossy().to_string();

        let html_content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("[WARN] Failed to read file {}: {}", path.display(), e);
                continue; // Skip this file if reading fails
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

    eprintln!("Finished document loading. Found {} final documents.", documents.len());
    Ok(documents)
}

/// Find the directory containing documentation for a specific crate
fn find_crate_doc_directory(target_doc_path: &Path, crate_name: &str) -> Result<PathBuf, DocLoaderError> {
    if !target_doc_path.exists() {
        return Err(DocLoaderError::DocNotFound(format!(
            "Documentation directory not found at {}. Please run cargo doc first.", 
            target_doc_path.display()
        )));
    }
    
    // First try the exact name match
    let exact_path = target_doc_path.join(crate_name);
    if exact_path.is_dir() && exact_path.join("index.html").exists() {
        return Ok(exact_path);
    }
    
    // Try case-insensitive match for crate names
    // Some crates use underscores in the package name but hyphens in the directory
    let normalized_crate_name = crate_name.replace('-', "_");
    
    // Search for directories that might match
    for entry in fs::read_dir(target_doc_path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let dir_name = entry.file_name();
            let dir_name_str = dir_name.to_string_lossy().to_string();
            
            // Try both normalized (with underscores) and original name
            if dir_name_str.eq_ignore_ascii_case(&normalized_crate_name) || 
               dir_name_str.eq_ignore_ascii_case(crate_name) {
                let index_path = entry.path().join("index.html");
                if index_path.exists() {
                    return Ok(entry.path());
                }
            }
        }
    }
    
    Err(DocLoaderError::DocNotFound(format!(
        "Documentation for crate '{}' not found in {}. Please run cargo doc first.", 
        crate_name, target_doc_path.display()
    )))
}