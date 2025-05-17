use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_basic_config_generation() -> Result<()> {
    let temp_dir = tempdir()?;
    let cargo_path = temp_dir.path().join("Cargo.toml");
    
    // Create a simple Cargo.toml file for testing
    let cargo_content = r#"
[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }
"#;
    
    fs::write(&cargo_path, cargo_content)?;
    
    // Run the generator
    let output = Command::new(env!("CARGO_BIN_EXE_gen-rust-docs-mcp-server-config"))
        .arg("test-config")
        .arg("--cargo-path")
        .arg(&cargo_path)
        .arg("--bin-path")
        .arg("test-path")
        .output()?;
    
    assert!(output.status.success(), "Config generator failed: {}", 
        String::from_utf8_lossy(&output.stderr));
    
    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;
    
    // Validate the structure
    assert!(config.is_object());
    assert!(config.get("mcpServers").is_some());
    assert!(config["mcpServers"].is_object());
    
    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();
    assert!(servers.len() >= 2, "Expected at least 2 servers, got {}", servers.len());
    
    // Check for serde
    let serde_server = servers.iter().find(|(key, _)| key.contains("serde"));
    assert!(serde_server.is_some(), "No serde server found");
    
    // Check for tokio
    let tokio_server = servers.iter().find(|(key, _)| key.contains("tokio"));
    assert!(tokio_server.is_some(), "No tokio server found");
    
    // Check tokio features
    let tokio_args = tokio_server.unwrap().1["args"].as_array().unwrap();
    let features_arg = tokio_args.iter().find(|arg| arg.as_str().unwrap_or("").starts_with("-F"));
    assert!(features_arg.is_some(), "No features specified for tokio");
    
    Ok(())
}

#[test]
fn test_complex_dependency_specs() -> Result<()> {
    let temp_dir = tempdir()?;
    let cargo_path = temp_dir.path().join("Cargo.toml");
    
    // Create a Cargo.toml with complex dependency specifications
    let cargo_content = r#"
[package]
name = "complex_test"
version = "0.1.0"
edition = "2021"

[dependencies]
# Simple version
simple = "1.0"

# Complex with features
complex = { version = "^2.0", features = ["feat1", "feat2"] }

# Dev dependency
[dev-dependencies]
dev_dep = "3.0"

# Build dependency
[build-dependencies]
build_dep = { version = "4.0", features = ["build_feat"] }
"#;
    
    fs::write(&cargo_path, cargo_content)?;
    
    // Run the generator
    let output = Command::new(env!("CARGO_BIN_EXE_gen-rust-docs-mcp-server-config"))
        .arg("complex-test")
        .arg("--cargo-path")
        .arg(&cargo_path)
        .arg("--bin-path")
        .arg("test-path")
        .output()?;
    
    assert!(output.status.success());
    
    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;
    
    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 4, "Expected 4 servers, got {}", servers.len());
    
    // Check for simple dependency
    let simple_server = servers.iter().find(|(key, _)| key.contains("simple"));
    assert!(simple_server.is_some(), "No simple server found");
    let simple_args = simple_server.unwrap().1["args"].as_array().unwrap();
    assert_eq!(simple_args.len(), 1, "Simple server should have 1 arg");
    
    // Check for complex dependency with features
    let complex_server = servers.iter().find(|(key, _)| key.contains("complex"));
    assert!(complex_server.is_some(), "No complex server found");
    let complex_args = complex_server.unwrap().1["args"].as_array().unwrap();
    assert!(complex_args.len() > 1, "Complex server should have multiple args");
    
    // Ensure both features are included
    let feature_args: Vec<&str> = complex_args
        .iter()
        .skip(1) // Skip the crate spec arg
        .map(|v| v.as_str().unwrap())
        .collect();
    
    assert!(feature_args.contains(&"-Ffeat1"), "Feature feat1 not found");
    assert!(feature_args.contains(&"-Ffeat2"), "Feature feat2 not found");
    
    // Check for dev dependency
    let dev_server = servers.iter().find(|(key, _)| key.contains("dev-dep"));
    assert!(dev_server.is_some(), "No dev dependency server found");
    
    // Check for build dependency
    let build_server = servers.iter().find(|(key, _)| key.contains("build-dep"));
    assert!(build_server.is_some(), "No build dependency server found");
    let build_args = build_server.unwrap().1["args"].as_array().unwrap();
    assert!(build_args.iter().any(|arg| arg.as_str().unwrap_or("").contains("build_feat")), 
            "Build feature not found");
    
    Ok(())
}

#[test]
fn test_output_file() -> Result<()> {
    let temp_dir = tempdir()?;
    let cargo_path = temp_dir.path().join("Cargo.toml");
    let output_path = temp_dir.path().join("config.json");
    
    // Create a simple Cargo.toml file
    let cargo_content = r#"
[package]
name = "output_test"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
"#;
    
    fs::write(&cargo_path, cargo_content)?;
    
    // Run the generator with output file
    let output = Command::new(env!("CARGO_BIN_EXE_gen-rust-docs-mcp-server-config"))
        .arg("output-test")
        .arg("--cargo-path")
        .arg(&cargo_path)
        .arg("--bin-path")
        .arg("test-path")
        .arg("--output")
        .arg(&output_path)
        .output()?;
    
    assert!(output.status.success());
    
    // Verify file was created
    assert!(output_path.exists(), "Output file was not created");
    
    // Read and parse the output file
    let output_content = fs::read_to_string(&output_path)?;
    let config: Value = serde_json::from_str(&output_content)?;
    
    // Validate structure
    assert!(config.get("mcpServers").is_some());
    
    Ok(())
}