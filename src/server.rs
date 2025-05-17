use rmcp::model::AnnotateAble;
use crate::{
    crate_discovery::{discover_available_crates, find_matching_crate_names},
    doc_loader::{self, Document},
    embeddings::{self, OPENAI_CLIENT, cosine_similarity, generate_embeddings},
    error::ServerError,
};
use async_openai::{
    Client as OpenAIClient,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs, CreateEmbeddingRequestArgs,
    },
};
use ndarray::Array1;
use rmcp::model::{LoggingMessageNotification, RawResource};
use rmcp::{
    Error as McpError, Peer, ServerHandler,
    model::{
        CallToolResult, Content, GetPromptRequestParam, GetPromptResult, Implementation,
        ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, LoggingLevel,
        LoggingMessageNotificationMethod, LoggingMessageNotificationParam, Notification,
        PaginatedRequestParam, ProtocolVersion, ReadResourceRequestParam, ReadResourceResult,
        Resource, ResourceContents, ServerCapabilities, ServerInfo, ServerNotification,
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

// Data structure to hold per-crate information
#[derive(Clone)]
struct CrateData {
    documents: Arc<Vec<Document>>,
    embeddings: Arc<Vec<(String, Array1<f32>)>>,
}

// --- Main Server Struct ---
#[derive(Clone)]
pub struct RustDocsServer {
    // Single mutex for the entire state to avoid partial locking issues
    state: Arc<Mutex<ServerState>>,
}

// State that will be protected by the mutex
struct ServerState {
    // Map of crate names to their documents and embeddings
    crates: HashMap<String, CrateData>,
    // Workspace path where target/doc is located
    workspace_path: PathBuf,
    // Optional features for cargo doc
    features: Option<Vec<String>>,
    // Flag for lazy loading of crates
    enable_lazy_loading: bool,
    // Set of available crates in target/doc
    available_crates: HashSet<String>,
    // MCP peer for sending logs
    peer: Option<Peer<RoleServer>>,
    // Server configuration information for displaying in get_info()
    config_info: String,
}

fn _create_resource_text(uri: &str, name: &str) -> Resource {
    RawResource::new(uri, name.to_string()).no_annotation()
}

impl RustDocsServer {
    // Create a new server instance with the provided configuration
    pub fn new(
        config_info: String,
        workspace_path: PathBuf,
        features: Option<Vec<String>>,
        enable_lazy_loading: bool,
    ) -> Result<Self, ServerError> {
        // Initialize the state
        let state = ServerState {
            crates: HashMap::new(),
            workspace_path: workspace_path.clone(),
            features: features.clone(),
            enable_lazy_loading,
            available_crates: HashSet::new(),
            peer: None,
            config_info,
        };

        let server = Self {
            state: Arc::new(Mutex::new(state)),
        };

        // Discover available crates if lazy loading is enabled
        if enable_lazy_loading {
            tokio::task::block_in_place(|| {
                futures::executor::block_on(async {
                    server.discover_crates(&workspace_path).await?;
                    Ok::<_, ServerError>(())
                })
            })?;
        }

        Ok(server)
    }

    // Discover available crates in the workspace
    async fn discover_crates(&self, workspace_path: &PathBuf) -> Result<(), ServerError> {
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

        // Set the available crates in a single mutex lock
        let mut state = self.state.lock().await;
        state.available_crates = discovered_crates;

        Ok(())
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

        let mut state = self.state.lock().await;
        state.crates.insert(crate_name, crate_data);

        Ok(())
    }

    // Check if a crate exists
    async fn has_crate(&self, crate_name: &str) -> bool {
        let state = self.state.lock().await;
        state.crates.contains_key(crate_name)
    }

    // Get list of all crates
    async fn list_crate_names(&self) -> Vec<String> {
        let state = self.state.lock().await;
        state.crates.keys().cloned().collect()
    }

    // Send log message via MCP notification
    async fn send_log(&self, level: LoggingLevel, message: String) {
        let peer_option = {
            let state = self.state.lock().await;
            state.peer.clone()
        };

        if let Some(peer) = peer_option {
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
    }

    // Load or generate embeddings for a crate
    pub async fn load_crate_data(
        &self,
        crate_name: &str,
        openai_client: &OpenAIClient<OpenAIConfig>,
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
        // Get the configuration from state with a single lock
        let (workspace_path, features) = {
            let state = self.state.lock().await;
            (state.workspace_path.clone(), state.features.clone())
        };

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

    // Try to lazy load a crate if it's available
    async fn try_lazy_load_crate(&self, crate_name: &str) -> Result<bool, ServerError> {
        // Check if lazy loading is enabled and crate is available in a single lock operation
        let (enable_lazy_loading, is_available) = {
            let state = self.state.lock().await;
            (
                state.enable_lazy_loading,
                state.available_crates.contains(crate_name),
            )
        };

        if !enable_lazy_loading || !is_available {
            return Ok(false);
        }

        // Get the OpenAI client
        let openai_client = OPENAI_CLIENT
            .get()
            .ok_or_else(|| ServerError::Config("OpenAI client not initialized".to_string()))?;

        // Load the documentation and embeddings
        let (documents, embeddings, loaded_from_cache, tokens, cost) =
            self.load_crate_data(crate_name, openai_client).await?;

        // Add the crate to the server
        let documents_len = documents.len();
        let embeddings_len = embeddings.len();

        self.add_crate(crate_name.to_string(), documents, embeddings)
            .await?;

        // Log status
        if loaded_from_cache {
            eprintln!(
                "Lazy loaded crate '{}' from cache with {} documents.",
                crate_name, documents_len
            );
        } else {
            eprintln!(
                "Lazy loaded crate '{}' with {} documents. Generated {} embeddings for {} tokens (Est. Cost: ${:.6}).",
                crate_name,
                documents_len,
                embeddings_len,
                tokens.unwrap_or(0),
                cost.unwrap_or(0.0)
            );
        }

        Ok(true)
    }

    // Set the MCP peer for sending logs
    pub async fn set_peer(&self, peer: Peer<RoleServer>) {
        let mut state = self.state.lock().await;
        state.peer = Some(peer);
    }
}

#[tool(tool_box)]
impl RustDocsServer {
    #[tool(
        description = "Query documentation for a Rust crate using semantic search and LLM summarization."
    )]
    async fn query_rust_docs(
        &self,
        #[tool(aggr)] args: QueryRustDocsArgs,
    ) -> Result<CallToolResult, McpError> {
        let question = &args.question;
        let crate_name = &args.crate_name;

        // First check if already loaded
        let crate_data = {
            let state = self.state.lock().await;
            state.crates.get(crate_name).cloned()
        };

        // If not found, try lazy loading
        let crate_data = match crate_data {
            Some(data) => data,
            None => {
                // Send log for lazy loading attempt
                self.send_log(
                    LoggingLevel::Info,
                    format!("Attempting lazy loading for crate '{}'", crate_name),
                )
                .await;

                match self.try_lazy_load_crate(crate_name).await {
                    Ok(true) => {
                        self.send_log(
                            LoggingLevel::Info,
                            format!("Successfully lazy loaded crate '{}'", crate_name),
                        )
                        .await;

                        // Now we need to fetch the crate data
                        let state = self.state.lock().await;
                        match state.crates.get(crate_name) {
                            Some(data) => data.clone(),
                            None => {
                                return Err(McpError::internal_error(
                                    format!(
                                        "Failed to get data for crate '{}' after loading",
                                        crate_name
                                    ),
                                    None,
                                ));
                            }
                        }
                    }
                    Ok(false) | Err(_) => {
                        // Try to find similar crates as suggestions
                        let available_crates = {
                            let state = self.state.lock().await;
                            state
                                .available_crates
                                .iter()
                                .cloned()
                                .collect::<Vec<String>>()
                        };
                        let similar_crates =
                            find_matching_crate_names(crate_name, &available_crates);
                        let crate_list = self.list_crate_names().await;

                        return Err(McpError::invalid_params(
                            format!("No documentation available for crate '{}'", crate_name),
                            Some(json!({
                                "available_crates": crate_list,
                                "requested_crate": crate_name,
                                "similar_crates": similar_crates
                            })),
                        ));
                    }
                }
            }
        };

        // Log received query via MCP
        self.send_log(
            LoggingLevel::Info,
            format!("Received query for crate '{}': {}", crate_name, question),
        )
        .await;

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
                let context_doc = crate_data
                    .documents
                    .iter()
                    .find(|doc| doc.path == best_path);

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

        // Get config info synchronously
        let config_info = tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let state = self.state.lock().await;
                state.config_info.clone()
            })
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
            .map(|name| _create_resource_text(&format!("crate://{}", name), name))
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
