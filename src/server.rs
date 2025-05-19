use crate::{
    doc_loader::{Document, self},
    embeddings::{OPENAI_CLIENT, cosine_similarity, Embedding},
    embedding_cache_service::EmbeddingCacheService,
    error::ServerError, // Keep ServerError for ::new()
};
use async_openai::{
    types::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs,
    },
    // Client as OpenAIClient, // Removed unused import
};
use rmcp::model::AnnotateAble; // Import trait for .no_annotation()
use rmcp::{
    Error as McpError,
    Peer,
    ServerHandler, // Import necessary rmcp items
    model::{
        CallToolResult,
        Content,
        GetPromptRequestParam,
        GetPromptResult,
        /* EmptyObject, ErrorCode, */ Implementation,
        ListPromptsResult, // Removed EmptyObject, ErrorCode
        ListResourceTemplatesResult,
        ListResourcesResult,
        LoggingLevel, // Uncommented ListToolsResult
        LoggingMessageNotification,
        LoggingMessageNotificationMethod,
        LoggingMessageNotificationParam,
        Notification,
        PaginatedRequestParam,
        ProtocolVersion,
        RawResource,
        /* Prompt, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole, */ // Removed Prompt types
        ReadResourceRequestParam,
        ReadResourceResult,
        Resource,
        ResourceContents,
        ServerCapabilities,
        ServerInfo,
        ServerNotification,
    },
    service::{RequestContext, RoleServer},
    tool,
};
use schemars::JsonSchema; // Import JsonSchema
use serde::Deserialize; // Import Deserialize
use serde_json::json;
use std::{/* borrow::Cow, */ env, sync::Arc}; // Removed borrow::Cow
use tokio::sync::Mutex;

// --- Argument Struct for the Tool ---

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryRustDocsArgs {
    #[schemars(description = "The specific question about the crate's API or usage.")]
    question: String,
    #[schemars(description = "The crate name to load documentation from (uses locally generated docs).")]
    crate_name: String,
}

// --- Main Server Struct ---

// No longer needs ServerState, holds data directly
#[derive(Clone)] // Add Clone for tool macro requirements
pub struct RustDocsServer {
    embedding_cache_service: Arc<EmbeddingCacheService>, // Embedding cache service
    peer: Arc<Mutex<Option<Peer<RoleServer>>>>, // Uses tokio::sync::Mutex
    startup_message: Arc<Mutex<Option<String>>>, // Keep the message itself
    startup_message_sent: Arc<Mutex<bool>>,     // Flag to track if sent (using tokio::sync::Mutex)
                                                // tool_name and info are handled by ServerHandler/macros now
}

impl RustDocsServer {
    // Updated constructor - simplified to only initialize cache service and messaging
    pub fn new(
        startup_message: String,
    ) -> Result<Self, ServerError> {
        // Get OpenAI API key from environment
        let openai_api_key = env::var("OPENAI_API_KEY")
            .map_err(|_| ServerError::MissingEnvVar("OPENAI_API_KEY".to_string()))?;
        
        // Initialize the embedding cache service
        let embedding_cache_service = EmbeddingCacheService::new(openai_api_key)?;
        
        // Keep ServerError for potential future init errors
        Ok(Self {
            embedding_cache_service: Arc::new(embedding_cache_service),
            peer: Arc::new(Mutex::new(None)), // Uses tokio::sync::Mutex
            startup_message: Arc::new(Mutex::new(Some(startup_message))), // Initialize message
            startup_message_sent: Arc::new(Mutex::new(false)), // Initialize flag to false
        })
    }

    // Helper function to send log messages via MCP notification (remains mostly the same)
    pub fn send_log(&self, level: LoggingLevel, message: String) {
        let peer_arc = Arc::clone(&self.peer);
        tokio::spawn(async move {
            let mut peer_guard = peer_arc.lock().await;
            if let Some(peer) = peer_guard.as_mut() {
                let params = LoggingMessageNotificationParam {
                    level,
                    logger: None,
                    data: serde_json::Value::String(message),
                };
                let log_notification: LoggingMessageNotification = Notification {
                    method: LoggingMessageNotificationMethod,
                    params,
                };
                let server_notification =
                    ServerNotification::LoggingMessageNotification(log_notification);
                if let Err(e) = peer.send_notification(server_notification).await {
                    eprintln!("Failed to send MCP log notification: {}", e);
                }
            } else {
                eprintln!("Log task ran but MCP peer was not connected.");
            }
        });
    }

    // Helper for creating simple text resources
    fn _create_resource_text(&self, uri: &str, name: &str) -> Resource {
        RawResource::new(uri, name.to_string()).no_annotation()
    }
    
    // Find all available crates in the cargo doc directory
    fn get_available_crates(&self) -> Vec<String> {
        let target_doc_path = std::path::Path::new("./target/doc");
        
        // If the doc directory doesn't exist, return empty list
        if !target_doc_path.exists() || !target_doc_path.is_dir() {
            return Vec::new();
        }
        
        // Read the directory and collect crate names
        match std::fs::read_dir(target_doc_path) {
            Ok(entries) => {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                    .filter_map(|entry| {
                        // Check if this directory has an index.html file (indicating a proper doc dir)
                        let path = entry.path();
                        let has_index = path.join("index.html").exists();
                        
                        if has_index {
                            entry.file_name().to_str().map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect()
            },
            Err(_) => Vec::new(),
        }
    }
}

// --- Tool Implementation ---

#[tool(tool_box)] // Add tool_box here as well, mirroring the example
// Tool methods go in a regular impl block
impl RustDocsServer {
    // Define the tool using the tool macro
    // Name removed; will be handled dynamically by overriding list_tools/get_tool
    /// Try to send the startup message if it hasn't been sent yet
    async fn try_send_startup_message(&self) {
        let mut sent_guard = self.startup_message_sent.lock().await;
        if !*sent_guard {
            let mut msg_guard = self.startup_message.lock().await;
            if let Some(message) = msg_guard.take() {
                self.send_log(LoggingLevel::Info, message);
                *sent_guard = true;
            }
            drop(msg_guard);
        }
        drop(sent_guard);
    }
    
    /// Load documentation and embeddings for a custom crate
    async fn load_custom_crate_docs(&self, crate_name: &str) -> Result<(String, Vec<Document>, Vec<(String, Embedding)>), McpError> {
        self.send_log(
            LoggingLevel::Info,
            format!("Loading local documentation for crate '{}'", crate_name),
        );
        
        // Load documents from cargo doc
        let docs = doc_loader::load_documents_from_cargo_doc(crate_name)
            .map_err(|e| McpError::internal_error(format!("Failed to load local documentation: {}", e), None))?;
            
        if docs.is_empty() {
            return Err(McpError::internal_error(
                format!("No documentation found for crate '{}'. Run 'cargo doc --package {}' first.", crate_name, crate_name), 
                None
            ));
        }
        
        // Use embedding cache service to get or generate embeddings
        let mut array_embeddings = Vec::new();
        self.send_log(
            LoggingLevel::Info,
            format!("Using embedding cache service for crate '{}'", crate_name),
        );
        
        for doc in &docs {
            // Get embedding from cache or generate new one
            match self.embedding_cache_service.get_embedding(&doc.content).await {
                Ok(embedding) => {
                    array_embeddings.push((doc.path.clone(), embedding));
                },
                Err(e) => {
                    return Err(McpError::internal_error(
                        format!("Failed to get embedding for document: {}", e), 
                        None
                    ));
                }
            }
        }
        
        Ok((crate_name.to_string(), docs, array_embeddings))
    }
    
    /// Find the best matching document for a given question embedding
    fn find_best_match<'a>(
        &self, 
        question_embedding: &Embedding,
        embeddings: &'a [(String, Embedding)],
    ) -> Option<(&'a str, f32)> {
        let question_vector = question_embedding.to_array();
        
        let mut best_match: Option<(&str, f32)> = None;
        for (path, doc_embedding) in embeddings {
            let doc_vector = doc_embedding.to_array();
            let score = cosine_similarity(question_vector.view(), doc_vector.view());
            if best_match.is_none() || score > best_match.unwrap().1 {
                best_match = Some((path, score));
            }
        }
        
        best_match
    }
    
    /// Generate a response using the LLM based on matched document context
    async fn generate_llm_response(
        &self,
        matched_doc: &Document,
        question: &str,
        crate_name: &str,
    ) -> Result<String, McpError> {
        let system_prompt = format!(
            "You are an expert technical assistant for the Rust crate '{}'. \
             Answer the user's question based *only* on the provided context. \
             If the context does not contain the answer, say so. \
             Do not make up information. Be clear, concise, and comprehensive providing example usage code when possible.",
            crate_name
        );
        
        let user_prompt = format!(
            "Context:\n---\n{}\n---\n\nQuestion: {}",
            matched_doc.content, question
        );

        let llm_model: String = env::var("LLM_MODEL")
            .unwrap_or_else(|_| "gpt-4o-mini-2024-07-18".to_string());
            
        let chat_request = CreateChatCompletionRequestArgs::default()
            .model(llm_model)
            .messages(vec![
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system_prompt)
                    .build()
                    .map_err(|e| {
                        McpError::internal_error(
                            format!("Failed to build system message: {}", e),
                            None,
                        )
                    })?
                    .into(),
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user_prompt)
                    .build()
                    .map_err(|e| {
                        McpError::internal_error(
                            format!("Failed to build user message: {}", e),
                            None,
                        )
                    })?
                    .into(),
            ])
            .build()
            .map_err(|e| {
                McpError::internal_error(
                    format!("Failed to build chat request: {}", e),
                    None,
                )
            })?;

        // Get the OpenAI client
        let client = OPENAI_CLIENT
            .get()
            .ok_or_else(|| McpError::internal_error("OpenAI client not initialized", None))?;
            
        let chat_response = client.chat().create(chat_request).await.map_err(|e| {
            McpError::internal_error(format!("OpenAI chat API error: {}", e), None)
        })?;

        Ok(chat_response
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone())
            .unwrap_or_else(|| "Error: No response from LLM.".to_string()))
    }

    #[tool(
        description = "Query documentation for a specific Rust crate using semantic search and LLM summarization."
    )]
    async fn query_rust_docs(
        &self,
        #[tool(aggr)] // Aggregate arguments into the struct
        args: QueryRustDocsArgs,
    ) -> Result<CallToolResult, McpError> {
        // Send startup message if not already sent
        self.try_send_startup_message().await;

        let question = &args.question;
        let crate_name = &args.crate_name;
        
        // Load documentation and embeddings for the specified crate
        let (crate_name, documents, embeddings) = self.load_custom_crate_docs(crate_name).await?;

        // Log received query via MCP
        self.send_log(
            LoggingLevel::Info,
            format!("Received query for crate '{}': {}", crate_name, question),
        );

        // Generate embedding for the question
        let question_embedding = self.embedding_cache_service.get_embedding(question).await
            .map_err(|e| McpError::internal_error(
                format!("Failed to get embedding for question: {}", e), 
                None
            ))?;

        // Find the best matching document
        let response_text = match self.find_best_match(&question_embedding, &embeddings) {
            Some((best_path, score)) => {
                eprintln!("Best match found: {} (score: {})", best_path, score);
                
                if let Some(doc) = documents.iter().find(|doc| doc.path == best_path) {
                    self.generate_llm_response(doc, question, &crate_name).await?
                } else {
                    "Error: Could not find content for best matching document.".to_string()
                }
            },
            None => "Could not find any relevant document context.".to_string(),
        };

        // Format and return the result
        Ok(CallToolResult::success(vec![Content::text(format!(
            "From {} docs: {}",
            crate_name, response_text
        ))]))
    }
}

// --- ServerHandler Implementation ---

#[tool(tool_box)] // Use imported tool macro directly
impl ServerHandler for RustDocsServer {
    fn get_info(&self) -> ServerInfo {
        // Define capabilities using the builder
        let capabilities = ServerCapabilities::builder()
            .enable_tools() // Enable tools capability
            .enable_logging() // Enable logging capability
            // Add other capabilities like resources, prompts if needed later
            .build();

        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05, // Use latest known version
            capabilities,
            server_info: Implementation {
                name: "rust-docs-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            // Provide instructions for using the server
            instructions: Some(
                "This server provides tools to query Rust crate documentation. \
                 Use the 'query_rust_docs' tool with a specific question and crate name to get information \
                 about the crate's API, usage, and examples, derived from its official documentation. \
                 The crate documentation must be locally generated first using 'cargo doc --package <crate_name>'.".to_string()
            ),
        }
    }

    // --- Placeholder Implementations for other ServerHandler methods ---
    // Implement these properly if resource/prompt features are added later.

    async fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // Get all available crates from the cargo doc directory
        let available_crates = self.get_available_crates();
        
        // Create resources for each available crate
        let resources = available_crates
            .iter()
            .map(|crate_name| self._create_resource_text(&format!("crate://{}", crate_name), crate_name))
            .collect();
            
        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        // Check if the URI matches our crate URI format
        if let Some(crate_name) = request.uri.strip_prefix("crate://") {
            // Check if this crate's documentation exists
            let available_crates = self.get_available_crates();
            
            if available_crates.contains(&crate_name.to_string()) {
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        crate_name,
                        &request.uri,
                    )],
                })
            } else {
                Err(McpError::resource_not_found(
                    format!("Crate documentation not found: {}. Run 'cargo doc --package {}' first.", crate_name, crate_name),
                    Some(json!({ "uri": request.uri })),
                ))
            }
        } else {
            Err(McpError::resource_not_found(
                format!("Invalid resource URI format: {}", request.uri),
                Some(json!({ "uri": request.uri })),
            ))
        }
    }

    async fn list_prompts(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            next_cursor: None,
            prompts: Vec::new(), // No prompts defined yet
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        Err(McpError::invalid_params(
            // Or prompt_not_found if that exists
            format!("Prompt not found: {}", request.name),
            None,
        ))
    }

    async fn list_resource_templates(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            next_cursor: None,
            resource_templates: Vec::new(), // No templates defined yet
        })
    }
}
