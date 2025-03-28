use crate::{doc_loader::Document, error::ServerError, state::ServerState};
use ndarray::Array1;
use rmcp::{
    model::{
        Implementation,
        // LogLevel, // Removed - Compiler cannot resolve
        LoggingLevel,
        LoggingMessageNotification,
        LoggingMessageNotificationMethod,
        LoggingMessageNotificationParam,
        Notification,
        ProtocolVersion,
        ServerCapabilitiesBuilder,
        ServerCapabilitiesBuilderState,
        ServerInfo,
        ServerNotification,
        Tool,
        // Unused/Removed:
        // ServerCapabilities,
        // ToolsCapability,
        // ConstString, // Not needed directly
    },
    service::RoleServer,
    Peer,
};
// use rmcp::model::ConstString; // Removed unused import
use std::borrow::Cow;
// use std::str::FromStr; // Removed unused import
use std::{env, sync::Arc}; // Keep Arc
use tokio::sync::Mutex; // Use tokio's Mutex // Needed for Tool fields
// Removed empty serde_json import
// If other items from serde_json are needed later, they can be added here.

// The main server struct
pub struct RustDocsServer {
    pub state: Arc<Mutex<ServerState>>, // Use tokio Mutex for state
    pub peer: Arc<Mutex<Option<Peer<RoleServer>>>>, // Use tokio Mutex for peer
    pub info: ServerInfo,
    pub tool_name: String,
    // Message to send once upon first client connection
    pub startup_message: Mutex<Option<String>>,
}

impl RustDocsServer {
    // Updated signature to accept documents and embeddings
    pub fn new(
        // Made pub
        crate_name: String,
        // docs_path: String, // Removed unused parameter
        documents: Vec<Document>,
        embeddings: Vec<(String, Array1<f32>)>,
        startup_message: String, // Add startup message parameter
    ) -> Result<Self, ServerError> {
        let tool_name = format!("query_rust_docs_{}", crate_name);
        // Define the tool
        let tool = Tool {
            name: Cow::from(tool_name.clone()), // Convert to Cow
            description: Cow::from(format!( // Convert to Cow
                "Query the official Rust documentation for the '{}' crate. Use this tool to retrieve detailed information about '{}’s API, including structs, traits, enums, constants, and functions. Ideal for answering technical questions about how to use '{}' in Rust projects, such as understanding specific methods, configuration options, or integration details. Additionally, leverage this tool to ensure accuracy of written code by verifying API usage and to resolve Clippy or lint errors by clarifying correct implementations. For example, use it for questions like \"How do I configure routing in {}?\", \"What does this {} struct do?\", \"Is this {} method call correct?\", or \"How do I fix a Clippy warning about {} in my code?\"",
                crate_name, crate_name, crate_name, crate_name, crate_name, crate_name, crate_name
            )),
            // Convert Value to Arc<Map<String, Value>>
            input_schema: Arc::new(serde_json::json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": format!(
                            "The specific question about the '{}' crate’s API or usage. Should be a clear, focused query about its functionality, such as \"What are the parameters of {}’s main struct?\", \"How do I use {} for async operations?\", or \"How do I resolve a Clippy error related to {}?\"",
                            crate_name, crate_name, crate_name, crate_name
                        )
                    },
                    "crate": {
                        "type": "string",
                        "description": format!(
                            "The name of the crate to query. Must match the current crate, which is '{}'. This ensures the question is routed to the correct documentation.",
                            crate_name
                        ),
                        "enum": [crate_name.clone()]
                    }
                },
                "required": ["question", "crate"]
            }).as_object().expect("Invalid JSON schema").clone()) // Convert Value to Map and clone
        };
        // Initialize ServerCapabilities using the builder to enable logging
        // Explicitly type the builder state transitions using const generics
        let capabilities = ServerCapabilitiesBuilder::<
            ServerCapabilitiesBuilderState<false, false, false, false, false>,
        >::default()
        .enable_tools() // -> State<false, false, false, false, true>
        .enable_logging() // -> State<false, true, false, false, true>
        .build();

        let info = ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities, // Assign the correctly initialized capabilities
            server_info: Implementation {
                name: "rust-docs-rmcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: None,
        };
        let tools_vec = vec![tool]; // Create the vector of tools

        let state = Arc::new(Mutex::new(ServerState {
            crate_name: crate_name.clone(),
            // docs_path, // Removed usage of commented-out field
            documents,
            embeddings,
            tools: tools_vec, // Pass the tools vector to state
        }));

        Ok(Self {
            state,
            peer: Arc::new(Mutex::new(None)), // Use tokio Mutex for peer
            info,
            tool_name,
            startup_message: Mutex::new(Some(startup_message)), // Initialize new field
        })
    }

    // Helper function to send log messages via MCP notification
    pub fn send_log(&self, level: LoggingLevel, message: String) {
        // Clone the Arc for the new task. This is cheap.
        let peer_arc = Arc::clone(&self.peer);

        // Spawn a background task to handle sending the log.
        // The task takes ownership of the cloned Arc and the message.
        tokio::spawn(async move {
            // Attempt to acquire the lock *inside* the spawned task.
            // Using lock().await will asynchronously wait for the lock.
            let mut peer_guard = peer_arc.lock().await; // Use .await for tokio::sync::Mutex

            // Check if the peer is Some and proceed
            if let Some(peer) = peer_guard.as_mut() {
                // Construct the params inside the task
                        let params = LoggingMessageNotificationParam {
                            level, // LogLevel likely implements Copy or Clone
                            logger: None,
                            data: serde_json::Value::String(message), // message is moved into the task
                        };

                        // Construct the notification inside the task
                        let log_notification: LoggingMessageNotification = Notification {
                            method: LoggingMessageNotificationMethod,
                            params,
                        };
                        let server_notification = ServerNotification::LoggingMessageNotification(log_notification);

                        // Send the notification and await the future *inside* the task and lock scope
                        if let Err(e) = peer.send_notification(server_notification).await {
                            eprintln!("Failed to send MCP log notification: {}", e);
                        }
                    } else {
                        // Optional: Log that the peer was None when the task ran
                        eprintln!("Log task ran but MCP peer was not connected.");
                    }
                    // MutexGuard is dropped here, releasing the lock
            // Note: tokio::sync::Mutex::lock() panics if the mutex is poisoned.
            // If explicit poison handling is needed, consider using try_lock() instead.
        });
        // The send_log function returns immediately after spawning the task.
    }
}
