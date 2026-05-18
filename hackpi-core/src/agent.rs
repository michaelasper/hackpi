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
    _workspace_root: &Path,
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

    // Write full output to a temp file in /tmp so we don't pollute the workspace.
    // Include a timestamp for uniqueness across concurrent tool calls.
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path =
        std::path::PathBuf::from(format!("/tmp/hackpi-truncated-{safe_id}-{timestamp}.txt"));
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

/// A pending tool call collected during SSE event processing.
/// If `parse_error` is `Some`, the tool call's input JSON was malformed
/// and `input` will be `Value::Null`. The `execute_pending_tool_calls`
/// method will return a `ToolResult::SystemError` instead of dispatching.
struct PendingToolCall {
    id: String,
    name: String,
    input: Value,
    parse_error: Option<String>,
}

/// Result of processing SSE events from the API stream.
struct SseEvents {
    text: String,
    pending_tool_calls: Vec<PendingToolCall>,
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
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
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
                        let (input, parse_error) = match serde_json::from_str(&current_tool_input) {
                            Ok(v) => (v, None),
                            Err(e) => {
                                let err_msg = format!(
                                        "Failed to parse tool '{}' input JSON: {e}\nRaw input: {current_tool_input}",
                                        current_tool_name,
                                    );
                                (Value::Null, Some(err_msg))
                            }
                        };
                        pending_tool_calls.push(PendingToolCall {
                            id: current_tool_id.clone(),
                            name: current_tool_name.clone(),
                            input,
                            parse_error,
                        });
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
                ApiEvent::Error(err) => {
                    tx.send(AgentEvent::Error(err)).ok();
                }
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
    /// If a `PendingToolCall` has a `parse_error`, a `ToolResult::SystemError`
    /// is returned directly without dispatching, so the LLM gets feedback
    /// about malformed JSON.
    async fn execute_pending_tool_calls(
        &self,
        pending_tool_calls: &[PendingToolCall],
        tx: &mpsc::UnboundedSender<AgentEvent>,
        signal: &tokio::sync::watch::Receiver<bool>,
    ) -> Option<Vec<ContentBlock>> {
        let mut tool_results: Vec<ContentBlock> = Vec::new();

        for pending_call in pending_tool_calls {
            // If JSON parsing failed, return SystemError directly without dispatching
            if let Some(error_msg) = &pending_call.parse_error {
                tx.send(AgentEvent::ToolCallStart {
                    id: pending_call.id.clone(),
                    name: pending_call.name.clone(),
                })
                .ok();

                tool_results.push(ContentBlock::tool_result(&pending_call.id, error_msg));

                tx.send(AgentEvent::ToolCallEnd {
                    id: pending_call.id.clone(),
                    result: ToolResult::SystemError {
                        message: error_msg.clone(),
                    },
                })
                .ok();
                continue;
            }

            tx.send(AgentEvent::ToolCallStart {
                id: pending_call.id.clone(),
                name: pending_call.name.clone(),
            })
            .ok();

            let ctx = ToolContext {
                workspace_root: self.workspace_root.clone(),
                conversation_id: String::new(),
                signal: signal.clone(),
            };

            let result = self
                .tools
                .dispatch(&pending_call.name, pending_call.input.clone(), &ctx)
                .await;

            let tool_result_for_event = match &result {
                Some(ToolResult::Success { content }) => {
                    let truncated = truncate_output(
                        content,
                        MAX_TOOL_RESULT_BYTES,
                        &pending_call.id,
                        &self.workspace_root,
                    );
                    tool_results.push(ContentBlock::tool_result(&pending_call.id, &truncated));
                    ToolResult::Success { content: truncated }
                }
                Some(ToolResult::SystemError { message }) => {
                    tool_results.push(ContentBlock::tool_result(&pending_call.id, message));
                    ToolResult::SystemError {
                        message: message.clone(),
                    }
                }
                Some(ToolResult::Timeout) => {
                    tool_results.push(ContentBlock::tool_result(
                        &pending_call.id,
                        "Tool execution timed out.",
                    ));
                    ToolResult::Timeout
                }
                Some(ToolResult::Cancelled) => {
                    tx.send(AgentEvent::Done).ok();
                    return None;
                }
                None => {
                    let msg = format!("Unknown tool: {}", pending_call.name);
                    tool_results.push(ContentBlock::tool_result(&pending_call.id, &msg));
                    ToolResult::SystemError { message: msg }
                }
            };

            tx.send(AgentEvent::ToolCallEnd {
                id: pending_call.id.clone(),
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
    use crate::types::ApiConfig;

    #[tokio::test]
    async fn test_malformed_tool_json_returns_system_error() {
        // A PendingToolCall with a parse_error should produce a SystemError
        // instead of silently using Value::Null.
        let api = ApiClient::new(ApiConfig::default()).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let (tx, _rx) = mpsc::unbounded_channel();
        let (_signal_tx, signal_rx) = tokio::sync::watch::channel(false);

        let pending = vec![PendingToolCall {
            id: "tool_1".into(),
            name: "bash".into(),
            input: Value::Null,
            parse_error: Some(
                "Failed to parse tool 'bash' input JSON: expected value at line 1 column 2\nRaw input: {broken}".into(),
            ),
        }];

        let result = agent
            .execute_pending_tool_calls(&pending, &tx, &signal_rx)
            .await;

        assert!(result.is_some(), "should return tool results");
        let tool_results = result.unwrap();
        assert_eq!(tool_results.len(), 1, "should have one tool result");

        match &tool_results[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
            } => {
                assert_eq!(tool_use_id, "tool_1", "should preserve tool_use_id");
                assert!(
                    content.contains("Failed to parse"),
                    "SystemError message should describe parse failure: {content}"
                );
                assert!(
                    content.contains("Raw input: {broken}"),
                    "SystemError should include raw input: {content}"
                );
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_valid_tool_json_passes_through() {
        // A PendingToolCall without a parse_error should be dispatched normally.
        // Note: this tool call will fail because the tool doesn't exist,
        // but it should be a SystemError for "unknown tool", not a parse error.
        let api = ApiClient::new(ApiConfig::default()).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let (tx, _rx) = mpsc::unbounded_channel();
        let (_signal_tx, signal_rx) = tokio::sync::watch::channel(false);

        let pending = vec![PendingToolCall {
            id: "tool_1".into(),
            name: "nonexistent_tool".into(),
            input: serde_json::json!({"key": "value"}),
            parse_error: None,
        }];

        let result = agent
            .execute_pending_tool_calls(&pending, &tx, &signal_rx)
            .await;

        assert!(result.is_some(), "should return tool results");
        let tool_results = result.unwrap();
        assert_eq!(tool_results.len(), 1, "should have one tool result");

        match &tool_results[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
            } => {
                assert_eq!(tool_use_id, "tool_1", "should preserve tool_use_id");
                assert!(
                    content.contains("Unknown tool"),
                    "should report unknown tool, not parse error: {content}"
                );
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn test_truncate_output_under_limit_passthrough() {
        let content = "hello world";
        let result = truncate_output(content, 1024, "tool_1", std::path::Path::new("/tmp"));

        assert_eq!(result, content);
    }

    #[test]
    fn test_truncate_output_at_exact_limit_passthrough() {
        let content = "a".repeat(100);
        let result = truncate_output(&content, 100, "tool_1", std::path::Path::new("/tmp"));

        assert_eq!(
            result, content,
            "content at exact limit should pass through"
        );
    }

    #[test]
    fn test_truncate_output_one_byte_over_truncates() {
        let content = "a".repeat(101);
        let result = truncate_output(&content, 100, "tool_1", std::path::Path::new("/tmp"));

        assert_ne!(result, content, "content over limit should be modified");
        assert!(
            result.contains("Output truncated"),
            "should mention truncation"
        );
    }

    #[test]
    fn test_truncate_output_over_limit_writes_full_content_to_disk() {
        let tool_id = "disk_test_unique";
        let content = "a".repeat(1000);
        let _result = truncate_output(&content, 100, tool_id, std::path::Path::new("/tmp"));

        // Find the temp file in /tmp with the expected pattern
        let entries = std::fs::read_dir("/tmp").unwrap();
        let tmp_file = entries
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| {
                        s.starts_with("hackpi-truncated-disk_test_unique-") && s.ends_with(".txt")
                    })
                    .unwrap_or(false)
            })
            .expect("temp file should exist in /tmp");
        let on_disk = std::fs::read_to_string(tmp_file.path()).unwrap();
        assert_eq!(on_disk, content, "temp file should contain full output");

        // Clean up
        let _ = std::fs::remove_file(tmp_file.path());
    }

    #[test]
    fn test_truncate_output_over_limit_clips_and_mentions_file() {
        let tool_id = "clip_test";
        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, tool_id, std::path::Path::new("/tmp"));

        assert!(result.len() < content.len());
        let expected_clip: String = "a".repeat(100);
        assert!(result.starts_with(&expected_clip));
        assert!(result.contains("Output truncated"));
        assert!(result.contains("/tmp/hackpi-truncated-clip_test-"));

        // Clean up temp file
        let entries = std::fs::read_dir("/tmp").unwrap();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-clip_test-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_sanitizes_tool_id() {
        let content = "a".repeat(1000);
        let result = truncate_output(
            &content,
            100,
            "../../etc/passwd",
            std::path::Path::new("/tmp"),
        );

        assert!(
            !result.contains("../../etc/passwd"),
            "path traversal chars should be removed"
        );
        assert!(result.contains("/tmp/hackpi-truncated-"), "safe path used");
        // After filtering "../../etc/passwd", remaining chars are "etcpasswd"
        assert!(
            result.contains("etcpasswd"),
            "sanitized tool_id should appear in message"
        );

        // Verify the file exists in /tmp
        let entries = std::fs::read_dir("/tmp").unwrap();
        let found = entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("hackpi-truncated-etcpasswd-") && name.ends_with(".txt")
        });
        assert!(found, "file should use sanitized tool_id chars in /tmp");

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-etcpasswd-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_unsafe_only_tool_id_falls_back() {
        let content = "a".repeat(1000);
        // tool_id with ONLY unsafe chars (dots and slashes)
        let result = truncate_output(&content, 100, "../", std::path::Path::new("/tmp"));

        assert!(
            !result.contains("../"),
            "unsafe-only tool_id should be replaced"
        );
        assert!(result.contains("unknown"), "should use 'unknown' fallback");
        assert!(
            result.contains("/tmp/hackpi-truncated-unknown-"),
            "should use 'unknown' fallback in path"
        );

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-unknown-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_empty_tool_id_uses_fallback() {
        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, "", std::path::Path::new("/tmp"));

        assert!(result.contains("unknown"));

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-unknown-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_non_ascii_safe_boundary() {
        // multi-byte UTF-8 chars: each 'é' is 2 bytes
        let content = "é".repeat(200);
        let result = truncate_output(&content, 100, "utf8_test", std::path::Path::new("/tmp"));

        // 100 bytes should cut at char boundary (floor to even: 100)
        assert!(
            result.contains("Output truncated"),
            "should truncate safely at char boundary"
        );
        // Should not panic from mid-char split

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-utf8_test-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    // ── build_assistant_message tests ──────────────────────────────────

    #[test]
    fn test_build_assistant_message_adds_message_for_non_empty_text() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "Hello, world!");

        assert_eq!(conversation.len(), 1);
        assert!(
            matches!(conversation[0].role, Role::Assistant),
            "expected Assistant role"
        );
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "Hello, world!");
            }
            _ => panic!("expected Text block"),
        }
    }

    #[test]
    fn test_build_assistant_message_does_not_add_for_empty_text() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hi")],
        }];
        Agent::build_assistant_message(&mut conversation, "");

        assert_eq!(conversation.len(), 1, "no new message for empty text");
    }

    #[test]
    fn test_build_assistant_message_adds_for_whitespace_text() {
        // build_assistant_message only checks is_empty(), so whitespace
        // is considered non-empty and will be added to the conversation.
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "   ");

        assert_eq!(
            conversation.len(),
            1,
            "whitespace-only text is still non-empty and should be added"
        );
    }

    #[test]
    fn test_build_assistant_message_multiple_calls() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "First");
        Agent::build_assistant_message(&mut conversation, "Second");

        assert_eq!(conversation.len(), 2);
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "First"),
            other => panic!("expected Text block, got {other:?}"),
        }
        match &conversation[1].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Second"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    // ── handle_step_stop_reason tests ─────────────────────────────────

    #[test]
    fn test_handle_step_stop_reason_end_turn_returns_true() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("end_turn".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_stop_returns_true() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("stop".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_other_reason_returns_false() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("tool_use".to_string());
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_none_returns_false() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason: Option<String> = None;
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_end_turn_sends_done_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("end_turn".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));

        // Should have received a Done event
        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(_) => panic!("expected Done event"),
            Err(_) => panic!("expected Done event, got empty channel"),
        }
    }

    #[test]
    fn test_handle_step_stop_reason_stop_sends_done_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("stop".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));

        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(_) => panic!("expected Done event"),
            Err(_) => panic!("expected Done event, got empty channel"),
        }
    }

    #[test]
    fn test_handle_step_stop_reason_other_does_not_send_done() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("tool_use".to_string());
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));

        assert!(
            rx.try_recv().is_err(),
            "should not send any event for tool_use stop reason"
        );
    }

    // ── Integration: truncation with empty conversation ───────────────

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
}
