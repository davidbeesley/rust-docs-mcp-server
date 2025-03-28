use crate::{
    embeddings::{cosine_similarity, OPENAI_CLIENT}, // Import from sibling module
    // error::ServerError, // Removed unused import
    server::RustDocsServer, // Import from sibling module
};
use rmcp::model::{LoggingMessageNotification, LoggingMessageNotificationMethod, LoggingMessageNotificationParam, ServerNotification}; // Import protocol types
use async_openai::types::CreateEmbeddingRequestArgs;
use async_openai::types::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs, // Use specific message args
    CreateChatCompletionRequestArgs,
    // ChatCompletionRequestMessage, // No longer needed directly
    // Role, // Role is inferred by the args type
};
use ndarray::Array1;
use rmcp::{
    Error as McpErrorData, // Use top-level Error alias
    model::{
        ClientRequest,
        Content, // Use the public Content type alias
        // CallToolRequest, // Remove unused import
        CallToolResult,
        ListToolsResult,
        ErrorCode,
        EmptyObject, // Import EmptyObject
        ServerResult,
        Notification
        // Value,          // Use serde_json::Value
    },
    service::{RequestContext, RoleServer, Service, ServiceRole},
    Peer,
};
use serde_json::{Value, Map}; // Import Value and Map from serde_json
use std::borrow::Cow;
// use std::future::Future; // Removed unused import
// use std::pin::Pin; // Removed unused import

// Removed custom RustDocsResponse enum

// Removed #[async_trait::async_trait]
impl Service<RoleServer> for RustDocsServer {
    // Removed incorrect associated type definition for Resp

    // Use async fn signature as required by the trait
    async fn handle_request( // Added async keyword
        &self,
        request: ClientRequest, // Use ClientRequest enum
        _context: RequestContext<RoleServer>,
    // Use async fn signature as required by the trait
    ) -> Result<ServerResult, McpErrorData> { // Reverted to ServerResult as Resp is defined in RoleServer
        // Remove Box::pin wrapper
            // Match on the ClientRequest enum variants
            match request {
                ClientRequest::ListToolsRequest(_list_tools_req) => {
                    // Parameters for ListToolsRequest might be needed later (e.g., for pagination)
                    // Get tools from state
                    let state = self.state.lock().await; // Use await for tokio::sync::Mutex
                    // Using ServerResult::empty(()) to satisfy type checker, as variants are unknown
                    // Attempting to return ListToolsResult directly, assuming Into<ServerResult> exists
                    let tools_result = ListToolsResult {
                        tools: state.tools.clone(),
                        next_cursor: None,
                    };
                    // Wrap the result in the correct ServerResult variant as suggested by compiler
                    Ok(ServerResult::ListToolsResult(tools_result))
            }
            ClientRequest::CallToolRequest(call_tool_req) => { // Variant holds Request<_, CallToolRequestParam>
                // Access fields via call_tool_req.params
                if call_tool_req.params.name != self.tool_name {
                     return Err(McpErrorData {
                        code: ErrorCode::METHOD_NOT_FOUND,
                        message: Cow::from(format!("Unknown tool: {}", call_tool_req.params.name)),
                        data: None,
                    });
                }

                // Access arguments via call_tool_req.params.arguments (Option<JsonObject>)
                let args_json = call_tool_req.params.arguments.ok_or_else(|| McpErrorData {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from("Missing arguments for callTool"),
                    data: None,
                })?;
                // Assuming JsonObject is Map<String, Value>
                let args: &Map<String, Value> = &args_json; // Borrow the map

                let question = args.get("question").and_then(Value::as_str);
                let crate_param = args.get("crate").and_then(Value::as_str);

                let (question, crate_param) = match (question, crate_param) {
                    (Some(q), Some(c)) => (q, c),
                    _ => {
                         return Err(McpErrorData { // Construct ErrorData manually
                            code: ErrorCode::INVALID_PARAMS,
                            message: Cow::from("Missing 'question' or 'crate' argument in callTool params"),
                            data: None,
                        });
                    }
                };

                // Use try_lock for Mutex to avoid await and potential deadlocks in sync context if needed,
                // but since handle_request is async, .lock().await is fine.
                // Handle PoisonError properly if it occurs.
                let state = self.state.lock().await;
                // TODO: Handle potential PoisonError from lock().await

                if crate_param != state.crate_name {
                     return Err(McpErrorData { // Construct ErrorData manually
                        code: ErrorCode::INVALID_PARAMS,
                        message: Cow::from(format!(
                            "This server only supports queries for '{}', not '{}'",
                            state.crate_name, crate_param
                        )),
                        data: None,
                    });
                }

                // Log received query via MCP
                self.send_log(rmcp::model::LoggingLevel::Info, format!("Received query for crate '{}': {}", crate_param, question)); // Revert to "info" string
    
                let client = OPENAI_CLIENT.get().ok_or_else(|| McpErrorData { // Construct ErrorData manually
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from("OpenAI client not initialized"),
                    data: None,
                })?;

                // Use map_err to convert OpenAIError into McpErrorData
                let question_embedding_request = CreateEmbeddingRequestArgs::default()
                    .model("text-embedding-3-small")
                    .input(question.to_string())
                    .build()
                    .map_err(|e| McpErrorData { // Construct ErrorData manually
                        code: ErrorCode::INTERNAL_ERROR,
                        message: Cow::from(format!("Failed to build embedding request: {}", e)),
                        data: None,
                    })?; // Keep ? as it now returns Result<_, McpErrorData>
                let question_embedding_response = client
                    .embeddings()
                    .create(question_embedding_request)
                    .await
                    .map_err(|e| McpErrorData { // Construct ErrorData manually
                        code: ErrorCode::INTERNAL_ERROR,
                        message: Cow::from(format!("OpenAI API error: {}", e)),
                        data: None,
                    })?; // Keep ? as it now returns Result<_, McpErrorData>
                let question_embedding =
                    question_embedding_response.data.first().ok_or_else(|| McpErrorData { // Construct ErrorData manually
                        code: ErrorCode::INTERNAL_ERROR,
                        message: Cow::from("Failed to get embedding for question"),
                        data: None,
                    })?;
                let question_vector = Array1::from(question_embedding.embedding.clone());

                let mut best_match: Option<(&str, f32)> = None;

                for (path, doc_embedding) in &state.embeddings {
                    let score = cosine_similarity(question_vector.view(), doc_embedding.view());
                    if best_match.is_none() || score > best_match.unwrap().1 {
                        best_match = Some((path, score));
                    }
                }

                let response_text = match best_match {
                    Some((best_path, _score)) => { // Score not used directly in prompt, but kept for potential future use
                        println!("Best match found: {}", best_path);
                        // Find the document content
                        let context_doc = state
                            .documents
                            .iter()
                            .find(|doc| doc.path == best_path);

                        if let Some(doc) = context_doc {
                            // Construct the prompt for the LLM
                            let system_prompt = format!(
                                "You are an expert assistant for the Rust crate '{}'. \
                                Answer the user's question based *only* on the provided context. \
                                If the context does not contain the answer, say so. \
                                Do not make up information. Be concise.",
                                state.crate_name
                            );
                            let user_prompt = format!(
                                "Context:\n---\n{}\n---\n\nQuestion: {}",
                                doc.content, question
                            );

                            // Build the chat completion request
                            let chat_request = CreateChatCompletionRequestArgs::default()
                                .model("gpt-3.5-turbo") // Or another suitable model
                                .messages(vec![
                                    ChatCompletionRequestSystemMessageArgs::default() // Use specific args type
                                        // .role(Role::System) // Role is implicit
                                        .content(system_prompt)
                                        .build()
                                        .map_err(|e| McpErrorData { // Manually map error
                                            code: ErrorCode::INTERNAL_ERROR,
                                            message: Cow::from(format!("Failed to build system message: {}", e)),
                                            data: None,
                                        })?
                                        .into(), // Convert to ChatCompletionRequestMessage enum variant
                                    ChatCompletionRequestUserMessageArgs::default() // Use specific args type
                                        // .role(Role::User) // Role is implicit
                                        .content(user_prompt)
                                        .build()
                                        .map_err(|e| McpErrorData { // Manually map error
                                            code: ErrorCode::INTERNAL_ERROR,
                                            message: Cow::from(format!("Failed to build user message: {}", e)),
                                            data: None,
                                        })?
                                        .into(), // Convert to ChatCompletionRequestMessage enum variant
                                ])
                                .build()
                                .map_err(|e| McpErrorData {
                                    code: ErrorCode::INTERNAL_ERROR,
                                    message: Cow::from(format!("Failed to build chat request: {}", e)),
                                    data: None,
                                })?;

                            // Call the OpenAI API
                            let chat_response = client
                                .chat()
                                .create(chat_request)
                                .await
                                .map_err(|e| McpErrorData {
                                    code: ErrorCode::INTERNAL_ERROR,
                                    message: Cow::from(format!("OpenAI chat API error: {}", e)),
                                    data: None,
                                })?;

                            // Extract the response content
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

                // Construct the final result using the LLM's response
                let tool_result = CallToolResult {
                    content: vec![Content::text(format!("From {} docs: {}", state.crate_name, response_text))], // Add prefix similar to Node version
                    is_error: Some(false),
                };
                // Wrap the result in the correct ServerResult variant
                Ok(ServerResult::CallToolResult(tool_result))
            }
            // Handle other ClientRequest variants if necessary, e.g., PingRequest
            ClientRequest::PingRequest(_) => {
                 self.send_log(rmcp::model::LoggingLevel::Info, "Received PingRequest".to_string()); // Revert to "info" string
                 // Respond with EmptyResult for Ping
                 Ok(ServerResult::EmptyResult(EmptyObject {})) // Use EmptyObject
            }
             _ => Err(McpErrorData { // Construct ErrorData manually
                code: ErrorCode::METHOD_NOT_FOUND,
                message: Cow::from("Unsupported client request variant"),
                data: None,
            }),
        }
        // Removed closing }) for Box::pin
    }

    // Use async fn signature as required by the trait
    async fn handle_notification( // Added async keyword
        &self,
        notification: <RoleServer as ServiceRole>::PeerNot,
    ) -> Result<(), McpErrorData> { // Return Result directly
        // Removed Box::pin wrapper
            println!("Received notification: {:?}", notification);
            Ok(())
        // Removed closing }) for Box::pin
    }

    fn get_peer(&self) -> Option<Peer<RoleServer>> {
        // Use blocking_lock() for tokio::sync::Mutex in a sync context
        self.peer.blocking_lock().clone()
    }

    fn set_peer(&mut self, peer: Peer<RoleServer>) {
        // Send startup message if it exists (only happens on first connection)
        // Use blocking_lock as this is a sync method
        let mut startup_msg_guard = self.startup_message.blocking_lock();
        if let Some(message) = startup_msg_guard.take() { // take() removes the value, leaving None
            // Use a temporary peer clone to send the log before setting the main peer
            let temp_peer = peer.clone();
            let params = LoggingMessageNotificationParam {
                level: rmcp::model::LoggingLevel::Info,
                logger: None,
                data: serde_json::Value::String(message),
            };
            let log_notification: LoggingMessageNotification = Notification {
                method: LoggingMessageNotificationMethod,
                params,
            };
            let server_notification = ServerNotification::LoggingMessageNotification(log_notification);

            // Send synchronously within the sync method context
            // Note: This blocks set_peer until the notification is sent.
            // Consider if spawning a task is better, but that requires more changes.
            if let Err(e) = futures::executor::block_on(temp_peer.send_notification(server_notification)) {
                 eprintln!("Failed to send startup log notification: {}", e);
            }
            // Drop the guard early after taking the message
            drop(startup_msg_guard);
        }

        // Set the peer for future use
        // Use blocking_lock() for tokio::sync::Mutex in a sync context
        *self.peer.blocking_lock() = Some(peer);
    }

    fn get_info(&self) -> <RoleServer as ServiceRole>::Info {
        self.info.clone()
    }
}