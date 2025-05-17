use crate::error::ServerError;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

/// Creates a temporary directory with a minimal Cargo.toml file and runs cargo doc
/// to generate documentation for specified dependencies.
///
/// This allows generating documentation for dependencies even when the main project
/// doesn't compile, by creating a minimal project that only depends on the target crates.
pub fn generate_docs_for_deps(
    original_cargo_toml_path: &Path,
    features: &Option<Vec<String>>,
) -> Result<PathBuf, ServerError> {
    // Create a temporary directory
    let temp_dir = TempDir::new()
        .map_err(|e| ServerError::Config(format!("Failed to create temporary directory: {}", e)))?;

    // Read the original Cargo.toml
    let cargo_toml_content = fs::read_to_string(original_cargo_toml_path)
        .map_err(|e| ServerError::Config(format!("Failed to read Cargo.toml: {}", e)))?;

    // Parse the TOML content
    let cargo_toml: toml::Value = toml::from_str(&cargo_toml_content)
        .map_err(|e| ServerError::Config(format!("Failed to parse Cargo.toml: {}", e)))?;

    // Extract just the dependencies section
    let dependencies = cargo_toml
        .get("dependencies")
        .ok_or_else(|| ServerError::Config("No dependencies found in Cargo.toml".to_string()))?;

    // Create a new minimal Cargo.toml with just those dependencies
    let mut new_cargo_toml = toml::map::Map::new();

    // Add package information
    let mut package = toml::map::Map::new();
    package.insert(
        "name".to_string(),
        toml::Value::String("docs_only".to_string()),
    );
    package.insert(
        "version".to_string(),
        toml::Value::String("0.1.0".to_string()),
    );
    package.insert(
        "edition".to_string(),
        toml::Value::String("2021".to_string()),
    );
    new_cargo_toml.insert("package".to_string(), toml::Value::Table(package));

    // Add dependencies
    new_cargo_toml.insert("dependencies".to_string(), dependencies.clone());

    // Write the new Cargo.toml to the temporary directory
    let new_cargo_toml_path = temp_dir.path().join("Cargo.toml");
    fs::write(&new_cargo_toml_path, new_cargo_toml.to_string())
        .map_err(|e| ServerError::Config(format!("Failed to write new Cargo.toml: {}", e)))?;

    // Create a minimal src/lib.rs file (needed for cargo doc to work)
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir)
        .map_err(|e| ServerError::Config(format!("Failed to create src directory: {}", e)))?;
    fs::write(
        src_dir.join("lib.rs"),
        "// Empty library for documentation generation",
    )
    .map_err(|e| ServerError::Config(format!("Failed to create lib.rs: {}", e)))?;

    // Run cargo doc
    run_cargo_doc(&temp_dir.path(), features)?;

    // Return the path to the generated documentation
    let doc_path = temp_dir.path().join("target").join("doc");

    // We need to keep the TempDir from being dropped, as it would delete the directory
    // One approach is to convert it to a PathBuf and leak the memory (acceptable for this use case)
    let _temp_dir_path = temp_dir.into_path();

    Ok(doc_path)
}

/// Runs cargo doc in the specified directory with optional features.
fn run_cargo_doc(dir: &Path, features: &Option<Vec<String>>) -> Result<(), ServerError> {
    use std::process::Command;

    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir).arg("doc").arg("--no-deps"); // Only document the dependencies, not their dependencies

    // Add features if specified
    if let Some(feat_list) = features {
        if !feat_list.is_empty() {
            cmd.arg("--features");
            cmd.arg(feat_list.join(","));
        }
    }

    let output = cmd
        .output()
        .map_err(|e| ServerError::Config(format!("Failed to run cargo doc: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ServerError::Config(format!("cargo doc failed: {}", stderr)));
    }

    Ok(())
}
