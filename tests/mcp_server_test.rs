use rustdocs_mcp_server::server::RustDocsServer;
use rmcp::model::{ProtocolVersion};
use rmcp::ServerHandler;
use std::{env, path::Path};

// Helper functions
fn setup_test_env() -> bool {
    if env::var("OPENAI_API_KEY").is_err() {
        println!("Skipping MCP server tests as OPENAI_API_KEY is not set");
        return false;
    }
    
    // Check for required documentation
    if !Path::new("./target/doc").exists() {
        println!("Skipping MCP server tests as ./target/doc doesn't exist");
        return false;
    }
    
    true
}

#[tokio::test]
async fn test_server_initialization() {
    // Test that the server initializes correctly
    let startup_message = "Test server initialization".to_string();
    let server_result = RustDocsServer::new(startup_message);
    
    match server_result {
        Ok(server) => {
            // Server should initialize successfully
            let server_info = server.get_info();
            
            // Check basic server info
            assert_eq!(server_info.server_info.name, "rust-docs-mcp-server");
            assert!(server_info.protocol_version == ProtocolVersion::V_2024_11_05);
            
            // Check capabilities
            let capabilities = server_info.capabilities;
            assert!(capabilities.tools.is_some(), "Server should have tools capability");
            assert!(capabilities.logging.is_some(), "Server should have logging capability");
        }
        Err(e) => {
            // If OPENAI_API_KEY isn't set, this is expected
            if env::var("OPENAI_API_KEY").is_err() {
                println!("Server initialization failed as expected without API key: {}", e);
            } else {
                panic!("Server should initialize successfully: {}", e);
            }
        }
    }
}

// Note: We can't test the query_rust_docs tool or get_available_crates directly since they're private
// In a real integration test, you would test through the MCP protocol

// A simpler test that verifies we can send logs
#[tokio::test]
async fn test_server_logging() {
    if !setup_test_env() {
        return;
    }
    
    // Create a server instance
    let startup_message = "Test server logging".to_string();
    let server = RustDocsServer::new(startup_message).expect("Server should initialize");
    
    // Test sending a log message
    // This doesn't verify the log was received, but at least confirms the method doesn't panic
    server.send_log(rmcp::model::LoggingLevel::Info, "Test log message".to_string());
}