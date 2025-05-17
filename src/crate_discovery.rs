use crate::error::ServerError;
use std::{
    collections::HashSet,
    fs,
    path::Path,
};

/// Discovers available crates in the target/doc directory
pub fn discover_available_crates(target_doc_path: &Path) -> Result<Vec<String>, ServerError> {
    let mut crates = HashSet::new();
    
    // If path doesn't exist, return empty list
    if !target_doc_path.exists() {
        return Ok(Vec::new());
    }
    
    // Read the target/doc directory
    let entries = fs::read_dir(target_doc_path)
        .map_err(|e| ServerError::Config(format!("Failed to read directory {}: {}", 
            target_doc_path.display(), e)))?;
    
    // Process each entry
    for entry in entries {
        let entry = entry.map_err(|e| ServerError::Config(format!("Failed to read directory entry: {}", e)))?;
        let path = entry.path();
        
        // Only consider directories
        if path.is_dir() {
            // Check if this is a crate directory (contains index.html)
            if path.join("index.html").exists() {
                // Get the directory name as the crate name
                if let Some(crate_name) = path.file_name() {
                    if let Some(name_str) = crate_name.to_str() {
                        crates.insert(name_str.to_string());
                    }
                }
            }
        }
    }
    
    // Convert HashSet to Vec and sort
    let mut crate_list: Vec<String> = crates.into_iter().collect();
    crate_list.sort();
    
    Ok(crate_list)
}

/// A utility function to normalize a crate name by converting hyphens to underscores
/// This is helpful because some crates use hyphens in their package name but underscores
/// in their Rust identifier name.
pub fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

/// Maps different versions or variants of a crate name to a canonical form
pub fn canonical_crate_name(name: &str) -> String {
    // For now, just normalize. In the future, this could have more rules
    normalize_crate_name(name)
}

/// Finds all available normalizations or variants of a crate name
pub fn find_matching_crate_names(crate_name: &str, available_crates: &[String]) -> Vec<String> {
    let normalized_name = normalize_crate_name(crate_name);
    let with_hyphens = normalized_name.replace('_', "-");
    
    available_crates.iter()
        .filter(|&c| {
            let c_lower = c.to_lowercase();
            let normalized_lower = normalized_name.to_lowercase();
            let hyphen_lower = with_hyphens.to_lowercase();
            
            c_lower == normalized_lower || c_lower == hyphen_lower ||
            c_lower.contains(&normalized_lower) || normalized_lower.contains(&c_lower)
        })
        .cloned()
        .collect()
}