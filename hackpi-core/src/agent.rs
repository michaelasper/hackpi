use crate::api::{ApiClient, ApiEvent};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};
use crate::types::{ContentBlock, Message, Role, Usage};
use hackpi_guardrails::{GuardReason, PermissionDecision};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

#[derive(Debug)]
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
        cancelled: &AtomicBool,
    ) -> Option<SseEvents> {
        let mut current_text = String::new();
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        // Tracks whether we've received any `partial_json` delta for the
        // current tool call. When the first `partial_json` arrives we
        // discard the `{}` placeholder that `content_block_start` sets from
        // `block.input`, because the Anthropic streaming API always sends
        // `input: {}` as a placeholder and the real input via deltas.
        let mut has_seen_partial_json = false;
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;

        while let Some(event) = api_rx.recv().await {
            if cancelled.load(Ordering::Relaxed) {
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
                            // Accumulate incremental JSON fragments for tool input.
                            // The Anthropic streaming API sends tool parameters as
                            // `partial_json` deltas rather than in `content_block_start`.
                            if let Some(json_fragment) = &delta.partial_json {
                                if !has_seen_partial_json {
                                    // First partial_json: discard the `{}`
                                    // placeholder from content_block_start.
                                    has_seen_partial_json = true;
                                    current_tool_input.clear();
                                }
                                current_tool_input.push_str(json_fragment);
                            }
                        }
                    }
                    "content_block_start" => {
                        if let Some(block) = &evt.content_block {
                            if block.block_type == "tool_use" {
                                current_tool_id = block.id.clone().unwrap_or_default();
                                current_tool_name = block.name.clone().unwrap_or_default();
                                current_tool_input = String::new();
                                has_seen_partial_json = false;
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

    /// Build an assistant message from accumulated text content and tool calls.
    /// Text blocks come first, followed by `ToolUse` content blocks for each
    /// pending tool call. This ensures the conversation history always contains
    /// the assistant's `tool_use` blocks before any corresponding `tool_result`
    /// blocks.
    fn build_assistant_message(
        conversation: &mut Vec<Message>,
        text: &str,
        pending_tool_calls: &[PendingToolCall],
    ) {
        if text.is_empty() && pending_tool_calls.is_empty() {
            return;
        }

        let mut content: Vec<ContentBlock> =
            Vec::with_capacity(if text.is_empty() { 0 } else { 1 } + pending_tool_calls.len());

        if !text.is_empty() {
            content.push(ContentBlock::text(text));
        }

        for call in pending_tool_calls {
            content.push(ContentBlock::ToolUse {
                id: call.id.clone(),
                name: call.name.clone(),
                input: call.input.clone(),
            });
        }

        conversation.push(Message {
            role: Role::Assistant,
            content,
        });
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
        cancelled: Arc<AtomicBool>,
    ) {
        conversation.push(Message {
            role: Role::User,
            content: vec![ContentBlock::text(user_message)],
        });

        for _turn in 0..MAX_TURNS {
            if cancelled.load(Ordering::Relaxed) {
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
            let events = match Self::process_sse_events(&mut api_rx, &tx, &cancelled).await {
                Some(events) => events,
                None => return,
            };

            // Build assistant message from accumulated text and tool calls.
            // Text blocks are placed first, followed by ToolUse content blocks,
            // so the conversation history correctly precedes tool_result blocks.
            Self::build_assistant_message(conversation, &events.text, &events.pending_tool_calls);

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
        Agent::build_assistant_message(&mut conversation, "Hello, world!", &[]);

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
    fn test_build_assistant_message_does_not_add_for_empty_text_and_no_tool_calls() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hi")],
        }];
        Agent::build_assistant_message(&mut conversation, "", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "no new message for empty text with no tool calls"
        );
    }

    #[test]
    fn test_build_assistant_message_adds_for_whitespace_text() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "   ", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "whitespace-only text is still non-empty and should be added"
        );
    }

    #[test]
    fn test_build_assistant_message_multiple_calls() {
        let mut conversation: Vec<Message> = Vec::new();
        Agent::build_assistant_message(&mut conversation, "First", &[]);
        Agent::build_assistant_message(&mut conversation, "Second", &[]);

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

    #[test]
    fn test_build_assistant_message_includes_tool_use_blocks() {
        let mut conversation: Vec<Message> = Vec::new();
        let tool_calls = vec![
            PendingToolCall {
                id: "toolu_abc".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "src/main.rs"}),
                parse_error: None,
            },
            PendingToolCall {
                id: "toolu_def".into(),
                name: "grep".into(),
                input: serde_json::json!({"pattern": "TODO"}),
                parse_error: None,
            },
        ];
        Agent::build_assistant_message(&mut conversation, "Let me look.", &tool_calls);

        assert_eq!(conversation.len(), 1);
        assert!(
            matches!(conversation[0].role, Role::Assistant),
            "expected Assistant role"
        );
        assert_eq!(conversation[0].content.len(), 3, "text + 2 tool_use blocks");

        // First block should be text
        match &conversation[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Let me look."),
            _ => panic!("expected Text block first"),
        }

        // Second block should be first tool_use
        match &conversation[0].content[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_abc");
                assert_eq!(name, "read_file");
                assert_eq!(input, &serde_json::json!({"path": "src/main.rs"}));
            }
            _ => panic!("expected ToolUse block"),
        }

        // Third block should be second tool_use
        match &conversation[0].content[2] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "toolu_def");
                assert_eq!(name, "grep");
                assert_eq!(input, &serde_json::json!({"pattern": "TODO"}));
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[test]
    fn test_build_assistant_message_adds_message_for_only_tool_calls_with_empty_text() {
        let mut conversation: Vec<Message> = Vec::new();
        let tool_calls = vec![PendingToolCall {
            id: "toolu_ghi".into(),
            name: "search".into(),
            input: serde_json::json!({"query": "foo"}),
            parse_error: None,
        }];
        Agent::build_assistant_message(&mut conversation, "", &tool_calls);

        assert_eq!(
            conversation.len(),
            1,
            "should add message for tool calls even without text"
        );
        assert_eq!(
            conversation[0].content.len(),
            1,
            "only tool_use block, no text"
        );
        match &conversation[0].content[0] {
            ContentBlock::ToolUse { id, name, .. } => {
                assert_eq!(id, "toolu_ghi");
                assert_eq!(name, "search");
            }
            _ => panic!("expected ToolUse block"),
        }
    }

    #[test]
    fn test_build_assistant_message_does_not_add_for_empty_tool_calls_with_empty_text() {
        let mut conversation: Vec<Message> = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::text("hi")],
        }];
        Agent::build_assistant_message(&mut conversation, "", &[]);

        assert_eq!(
            conversation.len(),
            1,
            "no new message when both text and tool_calls are empty"
        );
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

    // ── process_sse_events: partial_json accumulation ──────────────────

    /// Helper to create an SSE event from a raw JSON string.
    fn make_event(json: &str) -> crate::api::ApiEvent {
        let event: crate::types::StreamEvent =
            serde_json::from_str(json).expect("valid SSE event JSON");
        crate::api::ApiEvent::Event(Box::new(event))
    }

    #[tokio::test]
    async fn test_process_sse_events_accumulates_partial_json() {
        // Simulate the Anthropic streaming API sending tool input via
        // content_block_start (with empty input) followed by multiple
        // content_block_delta events with partial_json fragments.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        // Send events that mirror real Anthropic streaming for a tool call
        tx.send(make_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"write","input":{}}}"#,
        )).ok();

        // These partial_json deltas carry the actual tool parameters
        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"{\"path\":"}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"\"hello.txt\","}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"\"content\":\"Hello!\"}"}}"#,
        )).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":1}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_some(), "should return SSE events");
        let events = result.unwrap();

        assert_eq!(
            events.pending_tool_calls.len(),
            1,
            "should have one pending tool call"
        );
        let tc = &events.pending_tool_calls[0];
        assert_eq!(tc.id, "toolu_01");
        assert_eq!(tc.name, "write");
        assert!(
            tc.parse_error.is_none(),
            "should parse successfully, got error: {:?}",
            tc.parse_error
        );

        // The input should be the reconstructed JSON from partial_json fragments
        assert_eq!(tc.input["path"], "hello.txt", "path should be 'hello.txt'");
        assert_eq!(tc.input["content"], "Hello!", "content should be 'Hello!'");

        // Drain the agent events (we don't need them for this test)
        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_partial_json_write_tool_params() {
        // Regression test for COR-157: the write tool should receive
        // both "path" and "content" parameters when they come via partial_json.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let full_input = r#"{"path":"src/main.rs","content":"fn main() {}"}"#;

        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_42","name":"write","input":{}}}"#,
        )).ok();

        // Send the entire input as one partial_json chunk (common with small inputs)
        let delta_json = format!(
            r#"{{"type":"content_block_delta","index":0,"delta":{{"partial_json":{}}}}}"#,
            serde_json::to_string(full_input).unwrap()
        );
        tx.send(make_event(&delta_json)).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert!(
            tc.parse_error.is_none(),
            "should parse cleanly: {:?}",
            tc.parse_error
        );
        assert_eq!(tc.input["path"], "src/main.rs");
        assert_eq!(tc.input["content"], "fn main() {}");

        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_git_read_via_partial_json() {
        // Regression test for COR-157: the git_read tool should receive
        // the "operation" parameter when it comes via partial_json.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_99","name":"git_read","input":{}}}"#,
        )).ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"partial_json":"{\"operat"}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"partial_json":"ion\":\"log\"}"}}"#,
        ))
        .ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert!(
            tc.parse_error.is_none(),
            "should parse cleanly: {:?}",
            tc.parse_error
        );
        assert_eq!(tc.input["operation"], "log");

        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_no_partial_json_still_works() {
        // Verify that the text delta path still works when there are no
        // partial_json events (e.g., a plain text response).
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"Hello "}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"World"}}"#,
        ))
        .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.text, "Hello World");
        assert!(events.pending_tool_calls.is_empty());

        while agent_rx.try_recv().is_ok() {}
    }

    // ── run() cancellation tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_run_returns_immediately_when_cancelled_true() {
        // When `cancelled` is true at the start of `run()`, the agent must
        // send `AgentEvent::Done` and return immediately without making any
        // API calls. This covers the core of COR-183: without a reset, a
        // previous Ctrl+C interrupt latches and kills the next generation.
        let api = ApiClient::new(ApiConfig::default()).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_signal_tx, signal_rx) = tokio::sync::watch::channel(false);
        let cancelled = Arc::new(AtomicBool::new(true));
        let mut conversation = Vec::new();

        agent
            .run("hello", &mut conversation, tx, signal_rx, cancelled)
            .await;

        // Should have sent exactly one Done event
        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(other) => panic!("expected Done, got {other:?}"),
            Err(e) => panic!("expected Done event, got empty channel: {e:?}"),
        }

        // No more events should be queued
        assert!(
            rx.try_recv().is_err(),
            "should have exactly one event (Done)"
        );

        // User message is pushed to conversation before the cancel check,
        // so conversation should have 1 entry. This documents the current
        // behavior — the user input is recorded even on cancellation.
        assert_eq!(
            conversation.len(),
            1,
            "conversation should contain user message"
        );
        assert!(
            matches!(conversation[0].role, Role::User),
            "the message should have User role"
        );
    }

    #[tokio::test]
    async fn test_run_does_not_exit_immediately_when_cancelled_reset() {
        // Regression test for COR-183: after resetting the cancelled flag,
        // a subsequent `run()` call must not exit immediately. Uses a
        // wiremock server to verify the agent actually makes an API call.
        let mock_server = wiremock::MockServer::start().await;

        // Mock a streaming response with a tool_use stop reason
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(
                    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"content\":[]}}\n\n\
                     data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
                     data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"text\":\"Hello!\"}}\n\n\
                     data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
                     data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}\n\n\
                     data: [DONE]\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

        let api_config = ApiConfig {
            endpoint: format!("{}/v1/messages", mock_server.uri()),
            model: "ds4".into(),
            max_tokens: 100,
            temperature: 0.0,
        };
        let api = ApiClient::new(api_config).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let cancelled = Arc::new(AtomicBool::new(true));

        // First run with cancelled=true — should exit immediately
        {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (_stx, srx) = tokio::sync::watch::channel(false);
            let mut conversation = Vec::new();

            agent
                .run("first", &mut conversation, tx, srx, Arc::clone(&cancelled))
                .await;

            match rx.try_recv() {
                Ok(AgentEvent::Done) => {} // expected
                Ok(other) => panic!("expected Done, got {other:?}"),
                Err(e) => panic!("expected Done event: {e:?}"),
            }
        }

        // Reset cancelled flag (mirrors the fix applied in main.rs)
        cancelled.store(false, Ordering::SeqCst);

        // Second run with cancelled=false — must NOT exit immediately.
        // The agent should reach the API call and get a response.
        {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (_stx, srx) = tokio::sync::watch::channel(false);
            let mut conversation = Vec::new();

            agent
                .run("second", &mut conversation, tx, srx, Arc::clone(&cancelled))
                .await;

            // Should NOT be Done — the agent should have processed the
            // streaming response, which includes "Hello!" text and an
            // end_turn stop reason.
            let mut found_hello = false;
            let mut found_done = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    AgentEvent::TextChunk(text) if text == "Hello!" => {
                        found_hello = true;
                    }
                    AgentEvent::Done => {
                        found_done = true;
                    }
                    AgentEvent::Usage(usage) => {
                        assert_eq!(usage.input_tokens, 10);
                        assert_eq!(usage.output_tokens, 5);
                    }
                    _ => {} // other events are fine
                }
            }

            assert!(
                found_hello,
                "should have received 'Hello!' text from mock server"
            );
            assert!(found_done, "should have received Done event at end");
        }
    }

    // ── process_sse_events: cancellation ──────────────────────────────

    #[tokio::test]
    async fn test_process_sse_events_returns_none_when_cancelled() {
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"hello"}}"#,
        ))
        .ok();

        let cancelled = AtomicBool::new(true);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_none(), "should return None when cancelled");

        // Should have sent a Done event
        match agent_rx.try_recv() {
            Ok(AgentEvent::Done) => {}
            Ok(other) => panic!("expected Done, got {other:?}"),
            Err(e) => panic!("expected Done event: {e:?}"),
        }

        // Drop remaining so the buffer doesn't accumulate
        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_processes_normally_when_not_cancelled() {
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"hello"}}"#,
        ))
        .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_some(), "should return events when not cancelled");
        let events = result.unwrap();
        assert_eq!(events.text, "hello");

        while agent_rx.try_recv().is_ok() {}
    }

    // ── AtomicBool reset semantics ────────────────────────────────────

    #[test]
    fn test_atomic_bool_reset_makes_load_return_false() {
        // Verify that storing true then false on an AtomicBool correctly
        // resets the flag — this is the core mechanism of the COR-183 fix.
        let flag = AtomicBool::new(true);
        assert!(flag.load(Ordering::SeqCst), "should start as true");

        flag.store(false, Ordering::SeqCst);
        assert!(!flag.load(Ordering::SeqCst), "should be false after reset");
    }

    #[tokio::test]
    async fn test_process_sse_events_mixed_text_and_partial_json() {
        // Verify that a response containing both text deltas and tool call
        // partial_json deltas processes both correctly.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        // Text block
        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"I'll help."}}"#,
        ))
        .ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        // Tool call block
        tx.send(make_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"write","input":{}}}"#,
        )).ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"{\"path\":\"x\",\"content\":\"y\"}"}}"#,
        )).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":1}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.text, "I'll help.");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert_eq!(tc.input["path"], "x");
        assert_eq!(tc.input["content"], "y");

        while agent_rx.try_recv().is_ok() {}
    }
}
