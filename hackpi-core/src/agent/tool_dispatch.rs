use crate::tools::{ToolContext, ToolResult};
use crate::types::ContentBlock;
use tokio::sync::mpsc;

use super::state::{truncate_output, Agent, AgentEvent, PendingToolCall, MAX_TOOL_RESULT_BYTES};

impl Agent {
    /// Execute all pending tool calls, dispatching each and collecting results.
    /// Returns `None` if cancelled via `ToolResult::Cancelled`.
    /// If a `PendingToolCall` has a `parse_error`, a `ToolResult::SystemError`
    /// is returned directly without dispatching, so the LLM gets feedback
    /// about malformed JSON.
    pub(crate) async fn execute_pending_tool_calls(
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
                Some(ToolResult::CommandError { content, exit_code }) => {
                    let message = format!("Command exited with code {exit_code}\n{content}");
                    let truncated = truncate_output(
                        &message,
                        MAX_TOOL_RESULT_BYTES,
                        &pending_call.id,
                        &self.workspace_root,
                    );
                    tool_results.push(ContentBlock::tool_result(&pending_call.id, &truncated));
                    ToolResult::CommandError {
                        content: truncated,
                        exit_code: *exit_code,
                    }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiClient;
    use crate::tools::ToolRegistry;
    use crate::types::ApiConfig;
    use std::path::PathBuf;
    use std::sync::Arc;

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
            input: serde_json::Value::Null,
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

    // ── Integration: truncation with empty conversation ───────────────

    #[test]
    fn test_no_empty_assistant_messages_in_tool_results() {
        let mut conversation: Vec<crate::types::Message> = vec![crate::types::Message {
            role: crate::types::Role::User,
            content: vec![crate::types::ContentBlock::text("hello")],
        }];

        let tool_results = vec![ContentBlock::tool_result("tool_1", "result")];

        let before = conversation.len();
        conversation.push(crate::types::Message {
            role: crate::types::Role::User,
            content: tool_results,
        });

        assert_eq!(
            conversation.len(),
            before + 1,
            "should add exactly one message"
        );
        let has_empty_assistant = conversation.iter().any(|m| {
            if !matches!(m.role, crate::types::Role::Assistant) {
                return false;
            }
            m.content.iter().any(
                |c| matches!(c, crate::types::ContentBlock::Text { text, .. } if text.is_empty()),
            )
        });
        assert!(
            !has_empty_assistant,
            "no empty assistant messages should exist"
        );
    }
}
