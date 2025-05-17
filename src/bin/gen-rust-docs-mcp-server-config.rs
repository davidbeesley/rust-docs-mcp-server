use anyhow::Result;
use clap::Parser;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{env, fs, path::PathBuf, process::exit, str::FromStr};

/// Configuration style options
#[derive(Debug, Clone)]
enum ConfigStyle {
    /// Claude Desktop configuration style
    ClaudeDesktop,
    /// Roo configuration style with additional fields
    Roo,
}

impl FromStr for ConfigStyle {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude-desktop" => Ok(ConfigStyle::ClaudeDesktop),
            "roo" => Ok(ConfigStyle::Roo),
            _ => Err(format!("Unknown configuration style: {}", s))
        }
    }
}

/// Generate MCP server configuration for Rust crate documentation
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// The name of the configuration to generate (e.g., "claude-desktop" or "roo")
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
    #[allow(dead_code)]
    package: Option<Package>,
    dependencies: Option<Dependencies>,
    #[serde(rename = "dev-dependencies")]
    dev_dependencies: Option<Dependencies>,
    #[serde(rename = "build-dependencies")]
    build_dependencies: Option<Dependencies>,
}

#[derive(Debug, Deserialize)]
struct Package {
    #[allow(dead_code)]
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

    // Parse the config style from the command line argument
    let config_style = match ConfigStyle::from_str(&cli.config_name) {
        Ok(style) => style,
        Err(err) => {
            eprintln!("{}. Using claude-desktop style.", err);
            ConfigStyle::ClaudeDesktop
        }
    };

    // Create MCP server configuration
    let mcp_config = generate_mcp_config(&config_style, &bin_path, &cargo_toml)?;

    // Output the configuration
    if let Some(output_path) = cli.output {
        fs::write(output_path, mcp_config)?;
    } else {
        println!("{}", mcp_config);
    }

    Ok(())
}

fn find_bin_path(user_bin_path: Option<String>) -> Result<String> {
    user_bin_path.map(Ok).unwrap_or_else(|| {
        which::which("rustdocs_mcp_server")
            .map(|path| path.to_string_lossy().to_string())
            .or_else(|_| {
                eprintln!("Warning: rustdocs_mcp_server not found in PATH. Using 'rustdocs_mcp_server' as the command.");
                Ok("rustdocs_mcp_server".to_string())
            })
    })
}

fn generate_mcp_config(
    config_style: &ConfigStyle,
    bin_path: &str,
    cargo_toml: &CargoToml,
) -> Result<String> {
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

    // Add style-specific extra fields to each server configuration
    if let Some(servers_obj) = servers.as_object_mut() {
        for (_id, server_config) in servers_obj.iter_mut() {
            match config_style {
                ConfigStyle::ClaudeDesktop => {
                    // Claude Desktop style doesn't need additional fields
                }
                ConfigStyle::Roo => {
                    // Roo style includes additional fields
                    server_config["env"] = json!({
                        "OPENAI_API_KEY": "YOUR_OPENAI_API_KEY_HERE"
                    });
                    server_config["disabled"] = json!(false);
                    server_config["alwaysAllow"] = json!([]);
                }
            }
        }
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
    let servers_obj = servers.as_object_mut().expect("servers must be an object");

    for (name, version_value) in dependencies {
        // Skip invalid crates, local or git dependencies
        if name.starts_with('.') || name.contains('/') || name.contains('\\') {
            continue;
        }
        
        if matches!(version_value, Value::Object(obj) if obj.contains_key("path") || obj.contains_key("git")) {
            continue;
        }

        // Extract version and features
        let (version, features) = extract_version_and_features(version_value);

        // Build server ID and args
        let server_id = format!("rust-docs-{}-{}", name.replace('_', "-"), *index);
        *index += 1;
        
        // Create args: start with crate@version, add features if any
        let args = std::iter::once(format!("{}@{}", name, version))
            .chain(features.iter().map(|f| format!("-F{}", f)))
            .collect::<Vec<_>>();

        // Add server configuration
        servers_obj.insert(server_id, json!({
            "command": bin_path,
            "args": args
        }));
    }

    Ok(())
}

fn extract_version_and_features(version_value: &Value) -> (String, Vec<String>) {
    match version_value {
        // Simple version string: package = "1.0"
        Value::String(v) => (v.clone(), Vec::new()),
        
        // Complex dependency spec: package = { version = "1.0", features = ["feature1"] }
        Value::Object(obj) => {
            let version = obj.get("version")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| "*".to_string());
            
            let features = match obj.get("features") {
                Some(Value::Array(arr)) => arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                _ => Vec::new()
            };
            
            (version, features)
        },
        
        // Default for any other value type
        _ => ("*".to_string(), Vec::new()),
    }
}
