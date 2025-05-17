use crate::{
    doc_loader::Document,
    embeddings::{OPENAI_CLIENT, cosine_similarity},
    error::ServerError,
};
use async_openai::{
    types::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs, CreateEmbeddingRequestArgs,
    },
};
use ndarray::Array1;
use rmcp::model::AnnotateAble;
use rmcp::{
    Error as McpError,
    Peer,
    ServerHandler,
    model::{
        CallToolResult,
        Content,
        GetPromptRequestParam,
        GetPromptResult,
        Implementation,
        ListPromptsResult,
        ListResourceTemplatesResult,
        ListResourcesResult,
        LoggingLevel,
        LoggingMessageNotification,
        LoggingMessageNotificationMethod,
        LoggingMessageNotificationParam,
        Notification,
        PaginatedRequestParam,
        ProtocolVersion,
        RawResource,
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
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::Mutex;

// --- Argument Struct for the Tool ---

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryRustDocsArgs {
    #[schemars(description = "The specific question about the crate's API or usage.")]
    question: String,
    
    #[schemars(description = "Name of the Rust crate to query documentation for.")]
    crate_name: String,
}

// --- Main Server Struct ---

#[derive(Clone)]
pub struct RustDocsServer {
    // Map of crate names to their documents and embeddings
    crates: Arc<Mutex<HashMap<String, CrateData>>>,
    peer: Arc<Mutex<Option<Peer<RoleServer>>>>,
    startup_message: Arc<Mutex<Option<String>>>,
    startup_message_sent: Arc<Mutex<bool>>,
}

// Data structure to hold per-crate information
#[derive(Clone)]
pub struct CrateData {
    documents: Arc<Vec<Document>>,
    embeddings: Arc<Vec<(String, Array1<f32>)>>,
}

impl RustDocsServer {
    // Updated constructor that doesn't require a specific crate
    pub fn new(startup_message: String) -> Result<Self, ServerError> {
        Ok(Self {
            crates: Arc::new(Mutex::new(HashMap::new())),
            peer: Arc::new(Mutex::new(None)),
            startup_message: Arc::new(Mutex::new(Some(startup_message))),
            startup_message_sent: Arc::new(Mutex::new(false)),
        })
    }

    // Add a new crate to the server
    pub async fn add_crate(
        &self,
        crate_name: String,
        documents: Vec<Document>,
        embeddings: Vec<(String, Array1<f32>)>,
    ) -> Result<(), ServerError> {
        let crate_data = CrateData {
            documents: Arc::new(documents),
            embeddings: Arc::new(embeddings),
        };
        
        let mut crates = self.crates.lock().await;
        crates.insert(crate_name, crate_data);
        
        Ok(())
    }

    // Check if a crate exists
    pub async fn has_crate(&self, crate_name: &str) -> bool {
        let crates = self.crates.lock().await;
        crates.contains_key(crate_name)
    }

    // Get list of all crates
    pub async fn list_crate_names(&self) -> Vec<String> {
        let crates = self.crates.lock().await;
        crates.keys().cloned().collect()
    }

    // Helper function to send log messages via MCP notification
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
}

// --- Tool Implementation ---

#[tool(tool_box)]
impl RustDocsServer {
    #[tool(
        description = "Query documentation for a Rust crate using semantic search and LLM summarization."
    )]
    async fn query_rust_docs(
        &self,
        #[tool(aggr)]
        args: QueryRustDocsArgs,
    ) -> Result<CallToolResult, McpError> {
        // --- Send Startup Message (if not already sent) ---
        let mut sent_guard = self.startup_message_sent.lock().await;
        if !*sent_guard {
            let mut msg_guard = self.startup_message.lock().await;
            if let Some(message) = msg_guard.take() {
                self.send_log(LoggingLevel::Info, message);
                *sent_guard = true;
            }
            drop(msg_guard);
            drop(sent_guard);
        } else {
            drop(sent_guard);
        }

        let question = &args.question;
        let crate_name = &args.crate_name;

        // Validate that we have documentation for this crate
        let crates = self.crates.lock().await;
        let crate_data = match crates.get(crate_name) {
            Some(data) => data.clone(),
            None => {
                return Err(McpError::invalid_params(
                    format!("No documentation available for crate '{}'", crate_name),
                    Some(json!({ 
                        "available_crates": crates.keys().cloned().collect::<Vec<String>>(),
                        "requested_crate": crate_name
                    })),
                ));
            }
        };
        drop(crates); // Release lock

        // Log received query via MCP
        self.send_log(
            LoggingLevel::Info,
            format!("Received query for crate '{}': {}", crate_name, question),
        );

        // --- Embedding Generation for Question ---
        let client = OPENAI_CLIENT
            .get()
            .ok_or_else(|| McpError::internal_error("OpenAI client not initialized", None))?;

        let embedding_model: String =
            env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());
        let question_embedding_request = CreateEmbeddingRequestArgs::default()
            .model(embedding_model)
            .input(question.to_string())
            .build()
            .map_err(|e| {
                McpError::internal_error(format!("Failed to build embedding request: {}", e), None)
            })?;

        let question_embedding_response = client
            .embeddings()
            .create(question_embedding_request)
            .await
            .map_err(|e| McpError::internal_error(format!("OpenAI API error: {}", e), None))?;

        let question_embedding = question_embedding_response.data.first().ok_or_else(|| {
            McpError::internal_error("Failed to get embedding for question", None)
        })?;

        let question_vector = Array1::from(question_embedding.embedding.clone());

        // --- Find Best Matching Document ---
        let mut best_match: Option<(&str, f32)> = None;
        for (path, doc_embedding) in crate_data.embeddings.iter() {
            let score = cosine_similarity(question_vector.view(), doc_embedding.view());
            if best_match.is_none() || score > best_match.unwrap().1 {
                best_match = Some((path, score));
            }
        }

        // --- Generate Response using LLM ---
        let response_text = match best_match {
            Some((best_path, _score)) => {
                eprintln!("Best match found: {}", best_path);
                let context_doc = crate_data.documents.iter().find(|doc| doc.path == best_path);

                if let Some(doc) = context_doc {
                    let system_prompt = format!(
                        "You are an expert technical assistant for the Rust crate '{}'. \
                         Answer the user's question based *only* on the provided context. \
                         If the context does not contain the answer, say so. \
                         Do not make up information. Be clear, concise, and comprehensive providing example usage code when possible.",
                        crate_name
                    );
                    let user_prompt = format!(
                        "Context:\n---\n{}\n---\n\nQuestion: {}",
                        doc.content, question
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

                    let chat_response = client.chat().create(chat_request).await.map_err(|e| {
                        McpError::internal_error(format!("OpenAI chat API error: {}", e), None)
                    })?;

                    chat_response
                        .choices
                        .first()
                        .and_then(|choice| choice.message.content.clone())
                        .unwrap_or_else(|| "Error: No response from LLM.".to_string())
                } else {
                    "Error: Could not find content for best matching document.".to_string()
                }
            }
            None => "Could not find any relevant document context.".to_string(),
        };

        // --- Format and Return Result ---
        Ok(CallToolResult::success(vec![Content::text(format!(
            "From {} docs: {}",
            crate_name, response_text
        ))]))
    }
}

// --- ServerHandler Implementation ---

#[tool(tool_box)]
impl ServerHandler for RustDocsServer {
    fn get_info(&self) -> ServerInfo {
        // Define capabilities
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_logging()
            .build();

        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities,
            server_info: Implementation {
                name: "rust-docs-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "This server provides tools to query documentation for Rust crates. \
                 Use the 'query_rust_docs' tool with a specific question and crate name to get information \
                 about its API, usage, and examples, derived from its official documentation."
                .to_string()
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // Create a resource for each available crate
        let crate_names = self.list_crate_names().await;
        let resources = crate_names.iter()
            .map(|name| self._create_resource_text(&format!("crate://{}", name), name))
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
        // Check if it's a crate URI
        if let Some(crate_name) = request.uri.strip_prefix("crate://") {
            if self.has_crate(crate_name).await {
                return Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(crate_name, &request.uri)],
                });
            }
        }

        Err(McpError::resource_not_found(
            format!("Resource URI not found: {}", request.uri),
            Some(json!({ "uri": request.uri })),
        ))
    }

    async fn list_prompts(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            next_cursor: None,
            prompts: Vec::new(),
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        Err(McpError::invalid_params(
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
            resource_templates: Vec::new(),
        })
    }
}
