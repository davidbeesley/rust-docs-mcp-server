use crate::{
    crate_discovery::{discover_available_crates, find_matching_crate_names},
    doc_loader::{self, Document},
    embeddings::{self, OPENAI_CLIENT, cosine_similarity, generate_embeddings},
    error::ServerError,
};
use async_openai::types::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, CreateEmbeddingRequestArgs,
};
use ndarray::Array1;
use rmcp::model::AnnotateAble;
use rmcp::{
    Error as McpError, Peer, ServerHandler,
    model::{
        CallToolResult, Content, GetPromptRequestParam, GetPromptResult, Implementation,
        ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, LoggingLevel,
        LoggingMessageNotification, LoggingMessageNotificationMethod,
        LoggingMessageNotificationParam, Notification, PaginatedRequestParam, ProtocolVersion,
        RawResource, ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents,
        ServerCapabilities, ServerInfo, ServerNotification,
    },
    service::{RequestContext, RoleServer},
    tool,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{HashMap, HashSet},
    env,
    path::PathBuf,
    sync::Arc,
};
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

// Global server state singleton
struct ServerState {
    // Map of crate names to their documents and embeddings
    crates: Mutex<HashMap<String, CrateData>>,
    // Workspace path where target/doc is located
    workspace_path: Mutex<PathBuf>,
    // Optional features for cargo doc
    features: Mutex<Option<Vec<String>>>,
    // Flag for lazy loading of crates
    enable_lazy_loading: Mutex<bool>,
    // Set of available crates in target/doc
    available_crates: Mutex<HashSet<String>>,
    peer: Mutex<Option<Peer<RoleServer>>>,
    // Server configuration information for displaying in get_info()
    config_info: Mutex<String>,
}

// Static global instance
lazy_static::lazy_static! {
    static ref SERVER_STATE: Arc<ServerState> = Arc::new(ServerState {
        crates: Mutex::new(HashMap::new()),
        workspace_path: Mutex::new(PathBuf::new()),
        features: Mutex::new(None),
        enable_lazy_loading: Mutex::new(false),
        available_crates: Mutex::new(HashSet::new()),
        peer: Mutex::new(None),
        config_info: Mutex::new(String::new()),
    });
}

// Simple public-facing server wrapper that provides a tool interface
#[derive(Clone)]
pub struct RustDocsServer;

// Data structure to hold per-crate information
#[derive(Clone)]
pub struct CrateData {
    documents: Arc<Vec<Document>>,
    embeddings: Arc<Vec<(String, Array1<f32>)>>,
}

// No features hashing - we use only content hashing now

// This function is no longer needed - we use the global cache via embeddings::store_embedding_by_content

impl RustDocsServer {
    // Updated constructor to initialize the global state
    pub fn new(
        config_info: String,
        workspace_path: PathBuf,
        features: Option<Vec<String>>,
        enable_lazy_loading: bool,
    ) -> Result<Self, ServerError> {
        // Initialize the global state
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                // Set the basic configuration fields in the global state
                {
                    let mut config = SERVER_STATE.config_info.lock().await;
                    *config = config_info;
                }

                {
                    let mut path = SERVER_STATE.workspace_path.lock().await;
                    *path = workspace_path.clone();
                }

                {
                    let mut feat = SERVER_STATE.features.lock().await;
                    *feat = features.clone();
                }

                {
                    let mut lazy_loading = SERVER_STATE.enable_lazy_loading.lock().await;
                    *lazy_loading = enable_lazy_loading;
                }

                // Clear any existing crates
                {
                    let mut crates = SERVER_STATE.crates.lock().await;
                    crates.clear();
                }

                // Reset peer
                {
                    let mut peer = SERVER_STATE.peer.lock().await;
                    *peer = None;
                }

                // Discover available crates if lazy loading is enabled
                if enable_lazy_loading {
                    let target_doc_path = workspace_path.join("target").join("doc");
                    let discovered_crates = discover_available_crates(&target_doc_path)
                        .unwrap_or_default()
                        .into_iter()
                        .collect::<HashSet<String>>();

                    eprintln!(
                        "Discovered {} crates in target/doc: {:?}",
                        discovered_crates.len(),
                        discovered_crates
                    );

                    // Set the available crates
                    let mut avail_crates = SERVER_STATE.available_crates.lock().await;
                    *avail_crates = discovered_crates;
                } else {
                    // Clear the available crates if lazy loading is disabled
                    let mut avail_crates = SERVER_STATE.available_crates.lock().await;
                    avail_crates.clear();
                }
            })
        });

        // Return the server handle
        Ok(Self)
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

        let mut crates = SERVER_STATE.crates.lock().await;
        crates.insert(crate_name, crate_data);

        Ok(())
    }

    // Check if a crate exists
    pub async fn has_crate(&self, crate_name: &str) -> bool {
        let crates = SERVER_STATE.crates.lock().await;
        crates.contains_key(crate_name)
    }

    // Get list of all crates
    pub async fn list_crate_names(&self) -> Vec<String> {
        let crates = SERVER_STATE.crates.lock().await;
        crates.keys().cloned().collect()
    }

    // Helper function to send log messages via MCP notification
    pub fn send_log(&self, level: LoggingLevel, message: String) {
        // Use the global state
        tokio::spawn(async move {
            let mut peer_guard = SERVER_STATE.peer.lock().await;
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

    // No longer needed as we use content-based caching
    // This function has been removed

    // Helper method to load crate documentation and embeddings
    async fn load_crate_data(
        &self,
        crate_name: &str,
        openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    ) -> Result<
        (
            Vec<Document>,
            Vec<(String, Array1<f32>)>,
            bool,
            Option<usize>,
            Option<f64>,
        ),
        ServerError,
    > {
        // Always load the current documentation first
        let features = SERVER_STATE.features.lock().await.clone();
        let workspace_path = SERVER_STATE.workspace_path.lock().await.clone();

        eprintln!(
            "Loading docs for crate: {} (Features: {:?})",
            crate_name, features
        );

        // Load the documents
        let loaded_documents = doc_loader::load_documents(&workspace_path, crate_name)?;
        eprintln!(
            "Loaded {} documents for {}.",
            loaded_documents.len(),
            crate_name
        );

        // Prepare storage for embeddings
        let mut embeddings = Vec::new();
        let mut documents_needing_embedding = Vec::new();
        let mut reused_count = 0;

        // Try to get embeddings from global content hash cache
        for doc in &loaded_documents {
            // Get document embedding from the global cache using content hash
            if let Some(embedding_vec) = embeddings::get_embedding_by_content(&doc.content) {
                // Found in global cache - reuse
                embeddings.push((doc.path.clone(), Array1::from(embedding_vec)));
                reused_count += 1;
            } else {
                // Not found in cache - needs embedding
                documents_needing_embedding.push(doc.clone());
            }
        }

        eprintln!(
            "Reusing {} cached embeddings, generating {} new embeddings.",
            reused_count,
            documents_needing_embedding.len()
        );

        // If all documents have cached embeddings, return early
        if documents_needing_embedding.is_empty() {
            return Ok((loaded_documents, embeddings, true, None, None));
        }

        // Generate embeddings for documents not in cache
        let embedding_model: String =
            env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());

        eprintln!(
            "Generating embeddings for {} documents...",
            documents_needing_embedding.len()
        );
        let (new_embeddings, total_tokens) = generate_embeddings(
            openai_client,
            &documents_needing_embedding,
            &embedding_model,
        )
        .await?;

        // Store new embeddings in global cache
        eprintln!(
            "Storing {} new embeddings in global cache...",
            new_embeddings.len()
        );
        for (i, (_path, embedding)) in new_embeddings.iter().enumerate() {
            if let Some(doc) = documents_needing_embedding.get(i) {
                // Store in global cache
                if let Err(e) =
                    embeddings::store_embedding_by_content(&doc.content, &embedding.to_vec())
                {
                    eprintln!("Warning: Failed to store in global cache: {}", e);
                }
            }
        }

        // Calculate cost for the new embeddings
        let cost_per_million = 0.02;
        let estimated_cost = (total_tokens as f64 / 1_000_000.0) * cost_per_million;
        eprintln!(
            "Embedding generation cost for {} ({} tokens): ${:.6}",
            crate_name, total_tokens, estimated_cost
        );

        // Merge with reused embeddings
        embeddings.extend(new_embeddings);

        Ok((
            loaded_documents,
            embeddings,
            false,
            Some(total_tokens),
            Some(estimated_cost),
        ))
    }
}

// --- Tool Implementation ---

#[tool(tool_box)]
impl RustDocsServer {
    #[tool(description = "Query Rust crate documentation")]
    async fn list_crates(
        &self,
    ) -> String {
        "Hello".to_string()
    }

    // #[tool(
    //     description = "Query documentation for a Rust crate using semantic search and LLM summarization."
    // )]
    // async fn query_rust_docs(
    //     &self,
    //     #[tool(aggr)]
    //     args: QueryRustDocsArgs,
    // ) -> Result<CallToolResult, McpError> {

    //     let question = &args.question;
    //     let crate_name = &args.crate_name;

    //     // Check if we have this crate loaded already
    //     let crates = SERVER_STATE.crates.lock().await;
    //     let crate_data = match crates.get(crate_name) {
    //         Some(data) => {
    //             // We already have this crate loaded
    //             let result = data.clone();
    //             drop(crates); // Release lock early
    //             result
    //         },
    //         None => {
    //             // If lazy loading is enabled, try to load the crate now
    //             let enable_lazy_loading = *SERVER_STATE.enable_lazy_loading.lock().await;
    //             if enable_lazy_loading {
    //                 // Check if this crate is available in target/doc
    //                 let available_crates = SERVER_STATE.available_crates.lock().await;
    //                 let is_available = available_crates.contains(crate_name);
    //                 drop(available_crates);
    //
    //                 if is_available {
    //                     self.send_log(
    //                         LoggingLevel::Info,
    //                         format!("Lazy loading documentation for crate '{}'", crate_name),
    //                     );
    //
    //                     // Load the documentation and embeddings
    //                     let openai_client = OPENAI_CLIENT
    //                         .get()
    //                         .ok_or_else(|| McpError::internal_error("OpenAI client not initialized", None))?;
    //
    //                     // Release the lock while we load
    //                     drop(crates);
    //
    //                     let (documents, embeddings, _is_from_cache, _tokens, _cost) =
    //                         self.load_crate_data(crate_name, openai_client).await
    //                         .map_err(|e| McpError::internal_error(format!("Failed to load crate '{}': {}", crate_name, e), None))?;
    //
    //                     // Re-acquire the lock to add the loaded crate
    //                     let mut crates = SERVER_STATE.crates.lock().await;
    //
    //                     // Add the crate data
    //                     let crate_data = CrateData {
    //                         documents: Arc::new(documents),
    //                         embeddings: Arc::new(embeddings),
    //                     };
    //
    //                     crates.insert(crate_name.to_string(), crate_data.clone());
    //                     drop(crates);
    //
    //                     crate_data
    //                 } else {
    //                     // Crate is not available in target/doc
    //                     // Do a fuzzy match to suggest similar crates
    //                     let available_crates = SERVER_STATE.available_crates.lock().await;
    //                     let available_vec: Vec<String> = available_crates.iter().cloned().collect();
    //                     let similar_crates = find_matching_crate_names(crate_name, &available_vec);
    //                     drop(available_crates);
    //                     drop(crates);
    //
    //                     return Err(McpError::invalid_params(
    //                         format!("No documentation available for crate '{}'", crate_name),
    //                         Some(json!({
    //                             "available_crates": self.list_crate_names().await,
    //                             "requested_crate": crate_name,
    //                             "similar_crates": similar_crates
    //                         })),
    //                     ));
    //                 }
    //             } else {
    //                 // Lazy loading is disabled, return error
    //                 let available_crates: Vec<String> = crates.keys().cloned().collect();
    //                 drop(crates);
    //
    //                 return Err(McpError::invalid_params(
    //                     format!("No documentation available for crate '{}'", crate_name),
    //                     Some(json!({
    //                         "available_crates": available_crates,
    //                         "requested_crate": crate_name
    //                     })),
    //                 ));
    //             }
    //         }
    //     };

    //     // Log received query via MCP
    //     self.send_log(
    //         LoggingLevel::Info,
    //         format!("Received query for crate '{}': {}", crate_name, question),
    //     );

    //     // --- Embedding Generation for Question ---
    //     let client = OPENAI_CLIENT
    //         .get()
    //         .ok_or_else(|| McpError::internal_error("OpenAI client not initialized", None))?;

    //     let embedding_model: String =
    //         env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());
    //     let question_embedding_request = CreateEmbeddingRequestArgs::default()
    //         .model(embedding_model)
    //         .input(question.to_string())
    //         .build()
    //         .map_err(|e| {
    //             McpError::internal_error(format!("Failed to build embedding request: {}", e), None)
    //         })?;

    //     let question_embedding_response = client
    //         .embeddings()
    //         .create(question_embedding_request)
    //         .await
    //         .map_err(|e| McpError::internal_error(format!("OpenAI API error: {}", e), None))?;

    //     let question_embedding = question_embedding_response.data.first().ok_or_else(|| {
    //         McpError::internal_error("Failed to get embedding for question", None)
    //     })?;

    //     let question_vector = Array1::from(question_embedding.embedding.clone());

    //     // --- Find Best Matching Document ---
    //     let mut best_match: Option<(&str, f32)> = None;
    //     for (path, doc_embedding) in crate_data.embeddings.iter() {
    //         let score = cosine_similarity(question_vector.view(), doc_embedding.view());
    //         if best_match.is_none() || score > best_match.unwrap().1 {
    //             best_match = Some((path, score));
    //         }
    //     }

    //     // --- Generate Response using LLM ---
    //     let response_text = match best_match {
    //         Some((best_path, _score)) => {
    //             eprintln!("Best match found: {}", best_path);
    //             let context_doc = crate_data.documents.iter().find(|doc| doc.path == best_path);

    //             if let Some(doc) = context_doc {
    //                 let system_prompt = format!(
    //                     "You are an expert technical assistant for the Rust crate '{}'. \
    //                      Answer the user's question based *only* on the provided context. \
    //                      If the context does not contain the answer, say so. \
    //                      Do not make up information. Be clear, concise, and comprehensive providing example usage code when possible.",
    //                     crate_name
    //                 );
    //                 let user_prompt = format!(
    //                     "Context:\n---\n{}\n---\n\nQuestion: {}",
    //                     doc.content, question
    //                 );

    //                 let llm_model: String = env::var("LLM_MODEL")
    //                     .unwrap_or_else(|_| "gpt-4o-mini-2024-07-18".to_string());
    //                 let chat_request = CreateChatCompletionRequestArgs::default()
    //                     .model(llm_model)
    //                     .messages(vec![
    //                         ChatCompletionRequestSystemMessageArgs::default()
    //                             .content(system_prompt)
    //                             .build()
    //                             .map_err(|e| {
    //                                 McpError::internal_error(
    //                                     format!("Failed to build system message: {}", e),
    //                                     None,
    //                                 )
    //                             })?
    //                             .into(),
    //                         ChatCompletionRequestUserMessageArgs::default()
    //                             .content(user_prompt)
    //                             .build()
    //                             .map_err(|e| {
    //                                 McpError::internal_error(
    //                                     format!("Failed to build user message: {}", e),
    //                                     None,
    //                                 )
    //                             })?
    //                             .into(),
    //                     ])
    //                     .build()
    //                     .map_err(|e| {
    //                         McpError::internal_error(
    //                             format!("Failed to build chat request: {}", e),
    //                             None,
    //                         )
    //                     })?;

    //                 let chat_response = client.chat().create(chat_request).await.map_err(|e| {
    //                     McpError::internal_error(format!("OpenAI chat API error: {}", e), None)
    //                 })?;

    //                 chat_response
    //                     .choices
    //                     .first()
    //                     .and_then(|choice| choice.message.content.clone())
    //                     .unwrap_or_else(|| "Error: No response from LLM.".to_string())
    //             } else {
    //                 "Error: Could not find content for best matching document.".to_string()
    //             }
    //         }
    //         None => "Could not find any relevant document context.".to_string(),
    //     };

    //     // --- Format and Return Result ---
    //     Ok(CallToolResult::success(vec![Content::text(format!(
    //         "From {} docs: {}",
    //         crate_name, response_text
    //     ))]))
    // }
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

        // Get config info synchronously
        let config_info = tokio::task::block_in_place(|| {
            futures::executor::block_on(async { SERVER_STATE.config_info.lock().await.clone() })
        });

        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities,
            server_info: Implementation {
                name: "rust-docs-mcp-server".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(format!(
                "{}\n\nThis server provides tools to query documentation for Rust crates. \
                 Use the 'query_rust_docs' tool with a specific question and crate name to get information \
                 about its API, usage, and examples, derived from its official documentation.",
                config_info
            )),
        }
    }

    async fn list_resources(
        &self,
        _request: PaginatedRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // Create a resource for each available crate
        let crate_names = self.list_crate_names().await;
        let resources = crate_names
            .iter()
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
