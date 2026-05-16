use crate::api::{ApiClient, ApiEvent};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};
use crate::types::{ContentBlock, Message, Role, Usage};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

const MAX_TURNS: u32 = 25;
const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

pub enum AgentEvent {
    TextChunk(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta(String),
    ToolCallEnd { id: String, result: ToolResult },
    Done,
    Error(String),
    Usage(Usage),
}

pub struct Agent {
    api: ApiClient,
    tools: Arc<ToolRegistry>,
    system_prompt: String,
    workspace_root: PathBuf,
}

impl Agent {
    pub fn new(
        api: ApiClient,
        tools: Arc<ToolRegistry>,
        system_prompt: String,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            api,
            tools,
            system_prompt,
            workspace_root,
        }
    }

    pub async fn run(
        &self,
        user_message: &str,
        conversation: &mut Vec<Message>,
        tx: mpsc::UnboundedSender<AgentEvent>,
        signal: tokio::sync::watch::Receiver<bool>,
    ) {
        conversation.push(Message {
            role: Role::User,
            content: vec![ContentBlock::text(user_message)],
        });

        for turn in 0..MAX_TURNS {
            if *signal.borrow() {
                tx.send(AgentEvent::Done).ok();
                return;
            }

            let (api_tx, mut api_rx) = mpsc::unbounded_channel();

            let send_result = self
                .api
                .send_messages(
                    conversation,
                    &self.tools.all_schemas(),
                    &self.system_prompt,
                    api_tx,
                )
                .await;

            if let Err(e) = send_result {
                tx.send(AgentEvent::Error(format!("API error: {e}"))).ok();
                break;
            }

            let mut current_text = String::new();
            let mut pending_tool_calls: Vec<(String, String, Value)> = Vec::new();
            let mut current_tool_id = String::new();
            let mut current_tool_name = String::new();
            let mut current_tool_input = String::new();
            let mut stop_reason: Option<String> = None;
            let mut usage: Option<Usage> = None;

            while let Some(event) = api_rx.recv().await {
                if *signal.borrow() {
                    tx.send(AgentEvent::Done).ok();
                    return;
                }

                match event {
                    ApiEvent::Event(evt) => {
                        match evt.event_type.as_str() {
                            "content_block_delta" => {
                                if let Some(delta) = &evt.delta {
                                    if let Some(text) = &delta.text {
                                        current_text.push_str(text);
                                        tx.send(AgentEvent::TextChunk(text.clone())).ok();
                                    }
                                    if let Some(stop) = &delta.stop_reason {
                                        stop_reason = Some(stop.clone());
                                    }
                                }
                            }
                            "content_block_start" => {
                                if let Some(block) = &evt.content_block {
                                    if block.block_type == "tool_use" {
                                        current_tool_id = block.id.clone().unwrap_or_default();
                                        current_tool_name = block.name.clone().unwrap_or_default();
                                        current_tool_input = String::new();
                                        if let Some(input) = &block.input {
                                            current_tool_input = input.to_string();
                                        }
                                    }
                                }
                            }
                            "content_block_stop" => {
                                if !current_tool_id.is_empty() {
                                    let input: Value =
                                        serde_json::from_str(&current_tool_input)
                                            .unwrap_or(Value::Null);
                                    pending_tool_calls.push((
                                        current_tool_id.clone(),
                                        current_tool_name.clone(),
                                        input,
                                    ));
                                    current_tool_id.clear();
                                    current_tool_name.clear();
                                    current_tool_input.clear();
                                }
                            }
                            "message_delta" => {
                                if let Some(delta) = &evt.delta {
                                    if let Some(stop) = &delta.stop_reason {
                                        stop_reason = Some(stop.clone());
                                    }
                                }
                                if let Some(u) = &evt.usage {
                                    usage = Some(u.clone());
                                }
                            }
                            _ => {}
                        }
                    }
                    ApiEvent::Done => break,
                }
            }

            if !current_text.is_empty() {
                conversation.push(Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::text(&current_text)],
                });
            }

            if let Some(u) = usage {
                tx.send(AgentEvent::Usage(u)).ok();
            }

            if pending_tool_calls.is_empty() {
                tx.send(AgentEvent::Done).ok();
                return;
            }

            let mut tool_results: Vec<ContentBlock> = Vec::new();

            for (tool_id, tool_name, tool_input) in &pending_tool_calls {
                tx.send(AgentEvent::ToolCallStart {
                    id: tool_id.clone(),
                    name: tool_name.clone(),
                })
                .ok();

                let ctx = ToolContext {
                    workspace_root: self.workspace_root.clone(),
                    conversation_id: String::new(),
                    signal: signal.clone(),
                };

                let result = self
                    .tools
                    .dispatch(tool_name, tool_input.clone(), &ctx)
                    .await;

                match &result {
                    Some(ToolResult::Success { content }) => {
                        let truncated = if content.len() > MAX_TOOL_RESULT_BYTES {
                            let mut clipped = content[..MAX_TOOL_RESULT_BYTES].to_string();
                            clipped.push_str("\n\n[Output truncated: ");
                            clipped.push_str(&format!("{} total bytes]", content.len()));
                            clipped
                        } else {
                            content.clone()
                        };
                        tool_results.push(ContentBlock::tool_result(tool_id, &truncated));
                    }
                    Some(ToolResult::SystemError { message }) => {
                        tool_results.push(ContentBlock::tool_result(tool_id, message));
                    }
                    Some(ToolResult::Timeout) => {
                        tool_results.push(ContentBlock::tool_result(
                            tool_id,
                            "Tool execution timed out.",
                        ));
                    }
                    Some(ToolResult::Cancelled) => {
                        tx.send(AgentEvent::Done).ok();
                        return;
                    }
                    None => {
                        tool_results.push(ContentBlock::tool_result(
                            tool_id,
                            format!("Unknown tool: {tool_name}"),
                        ));
                    }
                }

                tx.send(AgentEvent::ToolCallEnd {
                    id: tool_id.clone(),
                    result: result.unwrap_or(ToolResult::SystemError {
                        message: "Unknown tool".into(),
                    }),
                })
                .ok();
            }

            if !tool_results.is_empty() {
                if turn > 0 {
                    conversation.push(Message {
                        role: Role::Assistant,
                        content: vec![ContentBlock::text("")],
                    });
                }
                conversation.push(Message {
                    role: Role::User,
                    content: tool_results,
                });
            }

            let should_stop = match &stop_reason {
                Some(s) if s == "end_turn" || s == "stop" => true,
                _ => false,
            };

            if should_stop {
                tx.send(AgentEvent::Done).ok();
                return;
            }
        }

        tx.send(AgentEvent::TextChunk(
            "\n\n[Turn limit reached. Starting fresh on your next request.]".into(),
        ))
        .ok();
        tx.send(AgentEvent::Done).ok();
    }
}
