use crate::api::{ApiClient, ApiEvent};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};
use crate::types::{ContentBlock, Message, Role, Usage};
use hackpi_guardrails::{GuardReason, PermissionDecision};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

const MAX_TURNS: u32 = 25;
const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

pub(crate) fn truncate_output(
    content: &str,
    max_bytes: usize,
    tool_id: &str,
    workspace_root: &Path,
) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let safe_id: String = tool_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let safe_id = if safe_id.is_empty() {
        "unknown"
    } else {
        &safe_id
    };

    let tmp_path = workspace_root.join(format!(".truncated_{safe_id}.txt"));
    let write_ok = std::fs::write(&tmp_path, content).is_ok();

    let end = content.floor_char_boundary(max_bytes);
    let mut clipped = content[..end].to_string();
    if write_ok {
        clipped.push_str(&format!(
            "\n\n[Output truncated: {} total bytes. Full output written to {}]",
            content.len(),
            tmp_path.display()
        ));
    } else {
        clipped.push_str(&format!(
            "\n\n[Output truncated: {} total bytes. Could not write full output to disk.]",
            content.len(),
        ));
    }
    clipped
}

pub enum AgentEvent {
    TextChunk(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallEnd {
        id: String,
        result: ToolResult,
    },
    Done,
    Error(String),
    Usage(Usage),
    PermissionRequest {
        id: u64,
        reason: GuardReason,
        response: oneshot::Sender<PermissionDecision>,
    },
}

/// Result of processing SSE events from the API stream.
struct SseEvents {
    text: String,
    pending_tool_calls: Vec<(String, String, Value)>,
    stop_reason: Option<String>,
    usage: Option<Usage>,
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

    /// Process SSE events from the API response stream.
    /// Handles text deltas, tool call starts/stops, and message deltas.
    /// Returns `None` if cancelled via signal.
    async fn process_sse_events(
        api_rx: &mut mpsc::UnboundedReceiver<ApiEvent>,
        tx: &mpsc::UnboundedSender<AgentEvent>,
        signal: &tokio::sync::watch::Receiver<bool>,
    ) -> Option<SseEvents> {
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
                return None;
            }

            match event {
                ApiEvent::Event(evt) => match evt.event_type.as_str() {
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
                    "content_block_stop" if !current_tool_id.is_empty() => {
                        let input: Value =
                            serde_json::from_str(&current_tool_input).unwrap_or(Value::Null);
                        pending_tool_calls.push((
                            current_tool_id.clone(),
                            current_tool_name.clone(),
                            input,
                        ));
                        current_tool_id.clear();
                        current_tool_name.clear();
                        current_tool_input.clear();
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
                },
                ApiEvent::Done => break,
            }
        }

        Some(SseEvents {
            text: current_text,
            pending_tool_calls,
            stop_reason,
            usage,
        })
    }

    /// Build an assistant message from accumulated text content.
    fn build_assistant_message(conversation: &mut Vec<Message>, text: &str) {
        if !text.is_empty() {
            conversation.push(Message {
                role: Role::Assistant,
                content: vec![ContentBlock::text(text)],
            });
        }
    }

    /// Execute all pending tool calls, dispatching each and collecting results.
    /// Returns `None` if cancelled via `ToolResult::Cancelled`.
    async fn execute_pending_tool_calls(
        &self,
        pending_tool_calls: &[(String, String, Value)],
        tx: &mpsc::UnboundedSender<AgentEvent>,
        signal: &tokio::sync::watch::Receiver<bool>,
    ) -> Option<Vec<ContentBlock>> {
        let mut tool_results: Vec<ContentBlock> = Vec::new();

        for (tool_id, tool_name, tool_input) in pending_tool_calls {
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

            let tool_result_for_event = match &result {
                Some(ToolResult::Success { content }) => {
                    let truncated = truncate_output(
                        content,
                        MAX_TOOL_RESULT_BYTES,
                        tool_id,
                        &self.workspace_root,
                    );
                    tool_results.push(ContentBlock::tool_result(tool_id, &truncated));
                    ToolResult::Success { content: truncated }
                }
                Some(ToolResult::SystemError { message }) => {
                    tool_results.push(ContentBlock::tool_result(tool_id, message));
                    ToolResult::SystemError {
                        message: message.clone(),
                    }
                }
                Some(ToolResult::Timeout) => {
                    tool_results.push(ContentBlock::tool_result(
                        tool_id,
                        "Tool execution timed out.",
                    ));
                    ToolResult::Timeout
                }
                Some(ToolResult::Cancelled) => {
                    tx.send(AgentEvent::Done).ok();
                    return None;
                }
                None => {
                    let msg = format!("Unknown tool: {tool_name}");
                    tool_results.push(ContentBlock::tool_result(tool_id, &msg));
                    ToolResult::SystemError { message: msg }
                }
            };

            tx.send(AgentEvent::ToolCallEnd {
                id: tool_id.clone(),
                result: tool_result_for_event,
            })
            .ok();
        }

        Some(tool_results)
    }

    /// Check if the stop reason indicates conversation should end.
    /// Returns `true` if the turn should stop.
    fn handle_step_stop_reason(
        stop_reason: &Option<String>,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> bool {
        let should_stop = matches!(stop_reason, Some(s) if s == "end_turn" || s == "stop");
        if should_stop {
            tx.send(AgentEvent::Done).ok();
        }
        should_stop
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

        for _turn in 0..MAX_TURNS {
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

            // Process SSE events from the API stream
            let events = match Self::process_sse_events(&mut api_rx, &tx, &signal).await {
                Some(events) => events,
                None => return,
            };

            // Build assistant message from accumulated text
            Self::build_assistant_message(conversation, &events.text);

            // Report usage if available
            if let Some(u) = events.usage {
                tx.send(AgentEvent::Usage(u)).ok();
            }

            // If no tool calls, we're done
            if events.pending_tool_calls.is_empty() {
                tx.send(AgentEvent::Done).ok();
                return;
            }

            // Execute pending tool calls
            let tool_results = match self
                .execute_pending_tool_calls(&events.pending_tool_calls, &tx, &signal)
                .await
            {
                Some(results) => results,
                None => return,
            };

            // Push tool results back to conversation
            if !tool_results.is_empty() {
                conversation.push(Message {
                    role: Role::User,
                    content: tool_results,
                });
            }

            // Check if we should stop based on stop reason
            if Self::handle_step_stop_reason(&events.stop_reason, &tx) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_output_under_limit_passthrough() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_under");
        let _ = std::fs::create_dir_all(&dir);

        let content = "hello world";
        let result = truncate_output(content, 1024, "tool_1", &dir);

        assert_eq!(result, content);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_at_exact_limit_passthrough() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_exact");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(100);
        let result = truncate_output(&content, 100, "tool_1", &dir);

        assert_eq!(
            result, content,
            "content at exact limit should pass through"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_one_byte_over_truncates() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_over1");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(101);
        let result = truncate_output(&content, 100, "tool_1", &dir);

        assert_ne!(result, content, "content over limit should be modified");
        assert!(
            result.contains("Output truncated"),
            "should mention truncation"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_over_limit_writes_full_content_to_disk() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_file");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(1000);
        let _result = truncate_output(&content, 100, "tool_1", &dir);

        let tmp_path = dir.join(".truncated_tool_1.txt");
        assert!(tmp_path.exists(), "temp file should exist on disk");
        let on_disk = std::fs::read_to_string(&tmp_path).unwrap();
        assert_eq!(on_disk, content, "temp file should contain full output");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_over_limit_clips_and_mentions_file() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_clip");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, "tool_1", &dir);

        assert!(result.len() < content.len());
        let expected_clip: String = "a".repeat(100);
        assert!(result.starts_with(&expected_clip));
        assert!(result.contains("Output truncated"));
        assert!(result.contains(".truncated_tool_1.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_sanitizes_tool_id() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_sanitize");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, "../../etc/passwd", &dir);

        assert!(
            !result.contains("../../etc/passwd"),
            "path traversal chars should be removed"
        );
        assert!(result.contains(".truncated_"), "safe filename used");
        // After filtering "../../etc/passwd", remaining chars are "etcpasswd"
        let safe_path = dir.join(".truncated_etcpasswd.txt");
        assert!(
            safe_path.exists(),
            "file should use sanitized tool_id chars"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_unsafe_only_tool_id_falls_back() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_unsafe");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(1000);
        // tool_id with ONLY unsafe chars (dots and slashes)
        let result = truncate_output(&content, 100, "../", &dir);

        assert!(
            !result.contains("../"),
            "unsafe-only tool_id should be replaced"
        );
        assert!(result.contains("unknown"), "should use 'unknown' fallback");
        let unknown_path = dir.join(".truncated_unknown.txt");
        assert!(
            unknown_path.exists(),
            "file should use 'unknown' fallback for all-unsafe tool_id"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_output_empty_tool_id_uses_fallback() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_empty_id");
        let _ = std::fs::create_dir_all(&dir);

        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, "", &dir);

        assert!(result.contains("unknown"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_no_empty_assistant_messages_in_tool_results() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hello")],
        }];

        let tool_results = vec![ContentBlock::tool_result("tool_1", "result")];

        let before = conversation.len();
        conversation.push(Message {
            role: Role::User,
            content: tool_results,
        });

        assert_eq!(
            conversation.len(),
            before + 1,
            "should add exactly one message"
        );
        let has_empty_assistant = conversation.iter().any(|m| {
            if !matches!(m.role, Role::Assistant) {
                return false;
            }
            m.content
                .iter()
                .any(|c| matches!(c, ContentBlock::Text { text, .. } if text.is_empty()))
        });
        assert!(
            !has_empty_assistant,
            "no empty assistant messages should exist"
        );
    }

    #[test]
    fn test_truncate_output_non_ascii_safe_boundary() {
        let dir = std::env::temp_dir().join("hackpi_trunc_test_utf8");
        let _ = std::fs::create_dir_all(&dir);

        // multi-byte UTF-8 chars: each 'é' is 2 bytes
        let content = "é".repeat(200);
        let result = truncate_output(&content, 100, "tool_1", &dir);

        // 100 bytes should cut at char boundary (floor to even: 100)
        assert!(
            result.contains("Output truncated"),
            "should truncate safely at char boundary"
        );
        // Should not panic from mid-char split

        let _ = std::fs::remove_dir_all(&dir);
    }
}
