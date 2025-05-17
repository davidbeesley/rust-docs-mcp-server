use anyhow::Result;
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::PathBuf,
    process::exit,
};

/// Generate MCP server configuration for Rust crate documentation
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The name of the configuration to generate (e.g., "claude-desktop")
    #[arg()]
    config_name: String,

    /// Path to the Cargo.toml file (defaults to current directory)
    #[arg(short = 'p', long)]
    cargo_path: Option<PathBuf>,

    /// Path to the rustdocs_mcp_server binary (defaults to searching in PATH)
    #[arg(short = 'b', long)]
    bin_path: Option<String>,

    /// Output file (defaults to stdout)
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoToml {
    package: Option<Package>,
    dependencies: Option<Dependencies>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<Dependencies>,
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<Dependencies>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
}

// Dynamic dependencies map that handles both simple string versions and detailed dependency specs
type Dependencies = std::collections::BTreeMap<String, Value>;

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Get the Cargo.toml path
    let cargo_path = match cli.cargo_path {
        Some(path) => path,
        None => env::current_dir()?.join("Cargo.toml"),
    };

    if !cargo_path.exists() {
        eprintln!("Error: Cargo.toml not found at {}", cargo_path.display());
        exit(1);
    }

    // Get the rustdocs_mcp_server path
    let bin_path = find_bin_path(cli.bin_path)?;

    // Parse the Cargo.toml file
    let cargo_content = fs::read_to_string(&cargo_path)?;
    let cargo_toml: CargoToml = toml::from_str(&cargo_content)?;

    // Create MCP server configuration
    let mcp_config = generate_mcp_config(&cli.config_name, &bin_path, &cargo_toml)?;

    // Output the configuration
    if let Some(output_path) = cli.output {
        fs::write(output_path, mcp_config)?;
    } else {
        println!("{}", mcp_config);
    }

    Ok(())
}

fn find_bin_path(user_bin_path: Option<String>) -> Result<String> {
    if let Some(path) = user_bin_path {
        return Ok(path);
    }

    // Try to find the binary in PATH
    match which::which("rustdocs_mcp_server") {
        Ok(path) => Ok(path.to_string_lossy().to_string()),
        Err(_) => {
            // Return a basic command and warn the user
            eprintln!(
                "Warning: rustdocs_mcp_server not found in PATH. Using 'rustdocs_mcp_server' as the command."
            );
            Ok("rustdocs_mcp_server".to_string())
        }
    }
}

fn generate_mcp_config(_config_name: &str, bin_path: &str, cargo_toml: &CargoToml) -> Result<String> {
    let mut servers = json!({});
    let mut index = 0;

    // Process all types of dependencies
    if let Some(deps) = &cargo_toml.dependencies {
        process_dependencies(&mut servers, deps, bin_path, &mut index)?;
    }

    if let Some(deps) = &cargo_toml.dev_dependencies {
        process_dependencies(&mut servers, deps, bin_path, &mut index)?;
    }

    if let Some(deps) = &cargo_toml.build_dependencies {
        process_dependencies(&mut servers, deps, bin_path, &mut index)?;
    }

    // Create the final configuration
    let config = json!({
        "mcpServers": servers
    });

    Ok(serde_json::to_string_pretty(&config)?)
}

fn process_dependencies(
    servers: &mut Value,
    dependencies: &Dependencies,
    bin_path: &str,
    index: &mut usize,
) -> Result<()> {
    for (name, version_value) in dependencies {
        // Skip internal/local dependencies and any crates with invalid names
        if name.starts_with(".") || name.contains("/") || name.contains("\\") {
            continue;
        }

        // Extract version and features
        let (version, features) = extract_version_and_features(version_value);

        // Create a unique ID for the server (name-index)
        let server_id = format!("rust-docs-{}-{}", name.replace("_", "-"), index);
        *index += 1;

        // Create server configuration
        let mut server_config = json!({
            "command": bin_path,
            "args": [
                format!("{}@{}", name, version)
            ]
        });

        // Add features if any
        if !features.is_empty() {
            let feature_args: Vec<String> = features
                .iter()
                .map(|f| format!("-F{}", f))
                .collect();
            
            let mut args = server_config["args"].as_array().unwrap().clone();
            for feature_arg in feature_args {
                args.push(Value::String(feature_arg));
            }
            
            server_config["args"] = Value::Array(args);
        }

        // Add server to the configuration
        if let Some(servers_obj) = servers.as_object_mut() {
            servers_obj.insert(server_id, server_config);
        }
    }

    Ok(())
}

fn extract_version_and_features(version_value: &Value) -> (String, Vec<String>) {
    let mut version = "*".to_string();
    let mut features = Vec::new();

    match version_value {
        // Simple version string: package = "1.0"
        Value::String(v) => {
            version = v.clone();
        }
        // Complex dependency spec: package = { version = "1.0", features = ["feature1"] }
        Value::Object(obj) => {
            if let Some(v) = obj.get("version") {
                if let Some(v_str) = v.as_str() {
                    version = v_str.to_string();
                }
            }

            if let Some(f) = obj.get("features") {
                if let Some(f_array) = f.as_array() {
                    for feature in f_array {
                        if let Some(feature_str) = feature.as_str() {
                            features.push(feature_str.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }

    (version, features)
}