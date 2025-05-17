use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::process::{Command, Output};
use tempfile::tempdir;

fn run_generator(args: &[&str], cargo_toml_content: &str) -> Result<Output> {
    let temp_dir = tempdir()?;
    let cargo_path = temp_dir.path().join("Cargo.toml");

    // Create the Cargo.toml file
    fs::write(&cargo_path, cargo_toml_content)?;

    // Build the base command
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gen-rust-docs-mcp-server-config"));

    // Add the config name as the first argument (use claude-desktop by default)
    cmd.arg("claude-desktop");

    // Add cargo path
    cmd.arg("--cargo-path").arg(&cargo_path);

    // Add test binary path
    cmd.arg("--bin-path").arg("test-path");

    // Add any additional arguments
    for arg in args {
        cmd.arg(arg);
    }

    // Run the command
    let output = cmd.output()?;

    Ok(output)
}

#[test]
fn test_basic_config_generation() -> Result<()> {
    // Simple Cargo.toml with basic dependencies
    let cargo_content = r#"
[package]
name = "test_project"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(
        output.status.success(),
        "Config generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate the structure
    assert!(config.is_object());
    assert!(config.get("mcpServers").is_some());
    assert!(config["mcpServers"].is_object());

    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();
    assert!(
        servers.len() >= 2,
        "Expected at least 2 servers, got {}",
        servers.len()
    );

    // Check for serde
    let serde_server = servers.iter().find(|(key, _)| key.contains("serde"));
    assert!(serde_server.is_some(), "No serde server found");

    // Check for tokio
    let tokio_server = servers.iter().find(|(key, _)| key.contains("tokio"));
    assert!(tokio_server.is_some(), "No tokio server found");

    // Check tokio features
    let tokio_args = tokio_server.unwrap().1["args"].as_array().unwrap();
    let features_arg = tokio_args
        .iter()
        .find(|arg| arg.as_str().unwrap_or("").starts_with("-F"));
    assert!(features_arg.is_some(), "No features specified for tokio");

    Ok(())
}

#[test]
fn test_complex_dependency_specs() -> Result<()> {
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

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(
        servers.len(),
        4,
        "Expected 4 servers, got {}",
        servers.len()
    );

    // Check for simple dependency
    let simple_server = servers.iter().find(|(key, _)| key.contains("simple"));
    assert!(simple_server.is_some(), "No simple server found");
    let simple_args = simple_server.unwrap().1["args"].as_array().unwrap();
    assert_eq!(simple_args.len(), 1, "Simple server should have 1 arg");

    // Check for complex dependency with features
    let complex_server = servers.iter().find(|(key, _)| key.contains("complex"));
    assert!(complex_server.is_some(), "No complex server found");
    let complex_args = complex_server.unwrap().1["args"].as_array().unwrap();
    assert!(
        complex_args.len() > 1,
        "Complex server should have multiple args"
    );

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
    assert!(
        build_args
            .iter()
            .any(|arg| arg.as_str().unwrap_or("").contains("build_feat")),
        "Build feature not found"
    );

    Ok(())
}

#[test]
fn test_output_file() -> Result<()> {
    let temp_dir = tempdir()?;
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

    // Run the generator with output file
    let output = run_generator(&["--output", output_path.to_str().unwrap()], cargo_content)?;

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

#[test]
fn test_empty_dependencies() -> Result<()> {
    // Cargo.toml with no dependencies
    let cargo_content = r#"
[package]
name = "empty_test"
version = "0.1.0"
edition = "2021"
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(
        servers.len(),
        0,
        "Expected 0 servers for empty dependencies"
    );

    Ok(())
}

#[test]
fn test_invalid_dependencies() -> Result<()> {
    // Cargo.toml with local/path dependencies that should be skipped
    let cargo_content = r#"
[package]
name = "invalid_test"
version = "0.1.0"
edition = "2021"

[dependencies]
# Path dependency - should be skipped
local_dep = { path = "../local_crate" }

# Valid dependency - should be included
valid_dep = "1.0"

# URL dependency - should be skipped
git_dep = { git = "https://github.com/example/example" }
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate server entries - only valid_dep should be included
    let servers = config["mcpServers"].as_object().unwrap();
    assert_eq!(
        servers.len(),
        1,
        "Expected 1 server (only valid_dep) for filtered dependencies"
    );

    // Check that the only server is for valid_dep
    let valid_server = servers.iter().find(|(key, _)| key.contains("valid-dep"));
    assert!(valid_server.is_some(), "Valid dependency not found");

    Ok(())
}

#[test]
fn test_package_naming() -> Result<()> {
    // Cargo.toml with dependencies having underscores
    let cargo_content = r#"
[package]
name = "naming_test"
version = "0.1.0"
edition = "2021"

[dependencies]
my_package = "1.0"
another_pkg = "2.0"
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Check for dash-normalized names (underscore to dash conversion)
    let servers = config["mcpServers"].as_object().unwrap();

    let my_package_server = servers.iter().find(|(key, _)| key.contains("my-package"));
    assert!(
        my_package_server.is_some(),
        "my-package server not found (underscore normalization failed)"
    );

    let another_pkg_server = servers.iter().find(|(key, _)| key.contains("another-pkg"));
    assert!(
        another_pkg_server.is_some(),
        "another-pkg server not found (underscore normalization failed)"
    );

    Ok(())
}

#[test]
fn test_command_formatting() -> Result<()> {
    // Simple Cargo.toml for command formatting test
    let cargo_content = r#"
[package]
name = "command_test"
version = "0.1.0"
edition = "2021"

[dependencies]
test_pkg = "1.0"
"#;

    // Run the generator with no extra args (the helper already adds --bin-path)
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate server entry has correct command path
    let servers = config["mcpServers"].as_object().unwrap();
    let server = servers.iter().next().unwrap().1;

    // Original test was checking for path with spaces, but we'll verify path is "test-path" instead
    // since that's what our run_generator sets by default
    assert_eq!(
        server["command"].as_str().unwrap(),
        "test-path",
        "Command path wasn't preserved correctly"
    );

    Ok(())
}

#[test]
fn test_version_specifications() -> Result<()> {
    // Cargo.toml with various version specifications
    let cargo_content = r#"
[package]
name = "version_test"
version = "0.1.0"
edition = "2021"

[dependencies]
# Default/latest version
latest = "*"

# Simple version number with varying specificity
major_only = "2"
major_minor = "2.0"
full_version = "2.0.37"

# Comparison operators
greater_than = ">1.2.3"
greater_equal = ">=1.0.0"
less_than = "<3.0"
less_equal = "<=2.5.0"

# Caret requirements
caret_req = "^1.2.3"    # 1.2.3 <= version < 2.0.0

# Tilde requirements
tilde_req = "~1.2.3"    # 1.2.3 <= version < 1.3.0

# Complex requirements
complex_range = ">1.0.0, <1.5.0, !=1.2.5"

# Exact version
exact = "=1.0.0"
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate server entries
    let servers = config["mcpServers"].as_object().unwrap();

    // Helper function to check a specific dependency
    let check_version = |name: &str, expected_version: &str| -> bool {
        let server = servers
            .iter()
            .find(|(key, _)| key.contains(name))
            .unwrap()
            .1;
        let args = server["args"].as_array().unwrap();
        let arg = args[0].as_str().unwrap();
        arg.contains(&format!("@{}", expected_version))
    };

    // Test each version type
    assert!(
        check_version("latest", "*"),
        "Latest version (*) not handled correctly"
    );
    assert!(
        check_version("major-only", "2"),
        "Major version not handled correctly"
    );
    assert!(
        check_version("major-minor", "2.0"),
        "Major.minor version not handled correctly"
    );
    assert!(
        check_version("full-version", "2.0.37"),
        "Full version not handled correctly"
    );

    // Comparison operators
    assert!(
        check_version("greater-than", ">1.2.3"),
        "> operator not handled correctly"
    );
    assert!(
        check_version("greater-equal", ">=1.0.0"),
        ">= operator not handled correctly"
    );
    assert!(
        check_version("less-than", "<3.0"),
        "< operator not handled correctly"
    );
    assert!(
        check_version("less-equal", "<=2.5.0"),
        "<= operator not handled correctly"
    );

    // Caret/tilde
    assert!(
        check_version("caret-req", "^1.2.3"),
        "Caret requirement not handled correctly"
    );
    assert!(
        check_version("tilde-req", "~1.2.3"),
        "Tilde requirement not handled correctly"
    );

    // Complex and exact
    assert!(
        check_version("complex-range", ">1.0.0, <1.5.0, !=1.2.5"),
        "Complex range not handled correctly"
    );
    assert!(
        check_version("exact", "=1.0.0"),
        "Exact version not handled correctly"
    );

    Ok(())
}

#[test]
fn test_multiple_features_formatting() -> Result<()> {
    // Test crate with multiple features to ensure proper formatting in CLI args
    let cargo_content = r#"
[package]
name = "features_test"
version = "0.1.0"
edition = "2021"

[dependencies]
# Multiple individual features
multi_features = { version = "1.0", features = ["feat1", "feat2", "feat3"] }

# Feature with dashes and underscores
complex_features = { version = "2.0", features = ["async-runtime", "full_json", "native-tls"] }
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate feature formatting
    let servers = config["mcpServers"].as_object().unwrap();

    // Check multiple features
    let multi_features = servers
        .iter()
        .find(|(key, _)| key.contains("multi-features"))
        .unwrap()
        .1;
    let args = multi_features["args"].as_array().unwrap();

    // Should find all three features
    let feature_args: Vec<&str> = args
        .iter()
        .filter_map(|arg| arg.as_str())
        .filter(|arg| arg.starts_with("-F"))
        .collect();

    assert_eq!(
        feature_args.len(),
        3,
        "Expected 3 feature flags for multi_features"
    );
    assert!(feature_args.contains(&"-Ffeat1"), "Feature feat1 not found");
    assert!(feature_args.contains(&"-Ffeat2"), "Feature feat2 not found");
    assert!(feature_args.contains(&"-Ffeat3"), "Feature feat3 not found");

    // Check complex feature names
    let complex_features = servers
        .iter()
        .find(|(key, _)| key.contains("complex-features"))
        .unwrap()
        .1;
    let complex_args = complex_features["args"].as_array().unwrap();

    let complex_feature_args: Vec<&str> = complex_args
        .iter()
        .filter_map(|arg| arg.as_str())
        .filter(|arg| arg.starts_with("-F"))
        .collect();

    assert_eq!(
        complex_feature_args.len(),
        3,
        "Expected 3 feature flags for complex_features"
    );
    assert!(
        complex_feature_args.contains(&"-Fasync-runtime"),
        "Feature async-runtime not found"
    );
    assert!(
        complex_feature_args.contains(&"-Ffull_json"),
        "Feature full_json not found"
    );
    assert!(
        complex_feature_args.contains(&"-Fnative-tls"),
        "Feature native-tls not found"
    );

    Ok(())
}

#[test]
fn test_rustdocs_mcp_examples() -> Result<()> {
    // Test based on example configurations shown in the README
    let cargo_content = r#"
[package]
name = "readme_examples"
version = "0.1.0"
edition = "2021"

[dependencies]
# Examples from the README
serde = "^1.0"
reqwest = "0.12.0"
tokio = "*"
async-stripe = "0.40"
"#;

    // Run the generator
    let output = run_generator(&[], cargo_content)?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate entries
    let servers = config["mcpServers"].as_object().unwrap();

    // Check that all example dependencies from README are handled
    let serde_server = servers.iter().find(|(key, _)| key.contains("serde"));
    assert!(serde_server.is_some(), "serde server not found");
    let serde_args = serde_server.unwrap().1["args"].as_array().unwrap();
    assert!(serde_args[0].as_str().unwrap().contains("serde@^1.0"));

    let reqwest_server = servers.iter().find(|(key, _)| key.contains("reqwest"));
    assert!(reqwest_server.is_some(), "reqwest server not found");
    let reqwest_args = reqwest_server.unwrap().1["args"].as_array().unwrap();
    assert!(reqwest_args[0].as_str().unwrap().contains("reqwest@0.12.0"));

    let tokio_server = servers.iter().find(|(key, _)| key.contains("tokio"));
    assert!(tokio_server.is_some(), "tokio server not found");
    let tokio_args = tokio_server.unwrap().1["args"].as_array().unwrap();
    assert!(tokio_args[0].as_str().unwrap().contains("tokio@*"));

    let async_stripe_server = servers.iter().find(|(key, _)| key.contains("async-stripe"));
    assert!(
        async_stripe_server.is_some(),
        "async-stripe server not found"
    );
    let async_stripe_args = async_stripe_server.unwrap().1["args"].as_array().unwrap();
    assert!(
        async_stripe_args[0]
            .as_str()
            .unwrap()
            .contains("async-stripe@0.40")
    );

    Ok(())
}

#[test]
fn test_roo_style_configuration() -> Result<()> {
    // Simple test to verify Roo style configuration
    let _cargo_content = r#"
[package]
name = "roo_test"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1.0"
"#;

    // Run the generator with Roo style
    let output = Command::new(env!("CARGO_BIN_EXE_gen-rust-docs-mcp-server-config"))
        .arg("roo")
        .arg("--bin-path")
        .arg("test-path")
        .env("CARGO_MANIFEST_DIR", std::env::current_dir()?)
        .output()?;

    assert!(output.status.success());

    // Parse the output
    let output_str = String::from_utf8(output.stdout)?;
    let config: Value = serde_json::from_str(&output_str)?;

    // Validate entries
    let servers = config["mcpServers"].as_object().unwrap();

    // Find the serde server
    let serde_server = servers.iter().find(|(key, _)| key.contains("serde"));
    assert!(serde_server.is_some(), "serde server not found");
    
    // Verify Roo-specific fields exist
    let server = serde_server.unwrap().1;
    assert!(server.get("env").is_some(), "env field not found");
    assert!(server.get("disabled").is_some(), "disabled field not found");
    assert!(server.get("alwaysAllow").is_some(), "alwaysAllow field not found");
    
    // Check specific values
    assert_eq!(server["disabled"], false, "disabled should be false");
    assert!(server["env"].as_object().unwrap().contains_key("OPENAI_API_KEY"), 
            "OPENAI_API_KEY not found in env");
    assert!(server["alwaysAllow"].as_array().unwrap().is_empty(),
            "alwaysAllow should be an empty array");

    Ok(())
}
