use crate::events::TuiEvent;
use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;
use hackpi_guardrails::{GuardReason, PermissionDecision};
use std::collections::VecDeque;

/// Represents a pending permission prompt awaiting user decision.
pub struct PermissionPrompt {
    pub id: u64,
    pub reason: GuardReason,
    pub response: Option<tokio::sync::oneshot::Sender<PermissionDecision>>,
}

impl std::fmt::Debug for PermissionPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionPrompt")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .field("response", &self.response.as_ref().map(|_| "Sender<..>"))
            .finish()
    }
}

pub enum AppState {
    Resting,
    Generating,
    Interrupted,
}

pub struct ConversationEntry {
    pub role: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallDisplay>,
}

pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub status: ToolCallStatus,
}

pub enum ToolCallStatus {
    Running,
    Done(ToolResult),
}

pub struct App {
    pub state: AppState,
    pub input: String,
    pub conversation: VecDeque<ConversationEntry>,
    pub scroll_offset: usize,
    pub usage: Option<Usage>,
    pub status_message: String,
    pub quit_requested: bool,
    pub pending_permission: Option<PermissionPrompt>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::Resting,
            input: String::new(),
            conversation: VecDeque::new(),
            scroll_offset: 0,
            usage: None,
            status_message: String::new(),
            quit_requested: false,
            pending_permission: None,
        }
    }

    pub fn handle_event(&mut self, event: TuiEvent) {
        match event {
            TuiEvent::Submit(text) => {
                self.conversation.push_back(ConversationEntry {
                    role: "user".into(),
                    text,
                    tool_calls: Vec::new(),
                });
                self.state = AppState::Generating;
                self.scroll_offset = 0;
            }
            TuiEvent::StreamChunk(chunk) => {
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        role: "assistant".into(),
                        text: chunk,
                        tool_calls: Vec::new(),
                    });
                } else if let Some(entry) = self.conversation.back_mut() {
                    entry.text.push_str(&chunk);
                }
            }
            TuiEvent::ToolCall { id, name } => {
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        role: "assistant".into(),
                        text: String::new(),
                        tool_calls: Vec::new(),
                    });
                }
                if let Some(entry) = self.conversation.back_mut() {
                    entry.tool_calls.push(ToolCallDisplay {
                        id,
                        name,
                        status: ToolCallStatus::Running,
                    });
                }
            }
            TuiEvent::ToolResult { id, result } => {
                if let Some(entry) = self.conversation.back_mut() {
                    for tc in &mut entry.tool_calls {
                        if tc.id == id {
                            tc.status = ToolCallStatus::Done(result);
                            break;
                        }
                    }
                }
            }
            TuiEvent::Usage(usage) => {
                self.usage = Some(usage);
            }
            TuiEvent::Error(err) => {
                self.status_message = err;
                self.state = AppState::Resting;
            }
            TuiEvent::Done => {
                self.state = AppState::Resting;
            }
            TuiEvent::PermissionRequest {
                id,
                reason,
                response,
            } => {
                self.pending_permission = Some(PermissionPrompt {
                    id,
                    reason,
                    response: Some(response),
                });
            }
        }
    }

    pub fn clear(&mut self) {
        self.conversation.clear();
        self.input.clear();
        self.usage = None;
        self.scroll_offset = 0;
    }
}

pub fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) -> bool {
    let parts: Vec<&str> = cmd.trim().splitn(2, char::is_whitespace).collect();
    let command = parts[0];
    match command {
        "/help" => {
            let help_text = "\
Available commands:
  /help  - Show this help message
  /clear - Clear the conversation
  /quit  - Exit the application";
            tui_tx
                .send(TuiEvent::StreamChunk(help_text.to_string()))
                .ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        "/clear" => {
            app.clear();
            true
        }
        "/quit" => {
            app.quit_requested = true;
            true
        }
        _ => {
            let err = format!("Unknown command: {command}. Type /help for available commands.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            true
        }
    }
}

/// Map a key character to a `PermissionDecision`, matching the key bindings
/// used in the TUI event loop when a permission prompt is active.
pub fn permission_decision_from_key(c: char) -> Option<PermissionDecision> {
    match c {
        '1' => Some(PermissionDecision::AllowOnce),
        '2' => Some(PermissionDecision::AllowSession),
        '3' => Some(PermissionDecision::Deny),
        '4' => Some(PermissionDecision::AlwaysAllow),
        '5' => Some(PermissionDecision::AlwaysDeny),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn test_slash_command_prevents_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let handled = handle_slash_command("/help", &mut app, &tx);
        assert!(handled);
    }

    #[test]
    fn test_slash_help_generates_help_text() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handled = handle_slash_command("/help", &mut app, &tx);
        assert!(handled);
        let mut found_chunk = false;
        let mut found_done = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::StreamChunk(text) => {
                    found_chunk = true;
                    assert!(text.contains("/help"));
                    assert!(text.contains("/clear"));
                    assert!(text.contains("/quit"));
                }
                TuiEvent::Done => found_done = true,
                _ => {}
            }
        }
        assert!(found_chunk);
        assert!(found_done);
    }

    #[test]
    fn test_slash_clear_clears_conversation() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        let handled = handle_slash_command("/clear", &mut app, &tx);
        assert!(handled);
        assert!(app.conversation.is_empty());
        assert!(app.input.is_empty());
    }

    #[test]
    fn test_unknown_slash_command_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handled = handle_slash_command("/unknown", &mut app, &tx);
        assert!(handled);
        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("/unknown"));
            }
        }
        assert!(found_error);
    }

    #[test]
    fn test_submit_creates_user_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "user");
        assert_eq!(app.conversation[0].text, "hello");
        assert!(matches!(app.state, AppState::Generating));
    }

    #[test]
    fn test_stream_chunk_appends_to_assistant() {
        let mut app = App::new();
        app.handle_event(TuiEvent::StreamChunk("Hello".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "assistant");
        assert_eq!(app.conversation[0].text, "Hello");

        app.handle_event(TuiEvent::StreamChunk(", world".into()));
        assert_eq!(app.conversation[0].text, "Hello, world");
    }

    #[test]
    fn test_tool_call_creates_assistant_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
        });
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "assistant");
        assert_eq!(app.conversation[0].tool_calls.len(), 1);
        assert_eq!(app.conversation[0].tool_calls[0].name, "read");
        assert!(matches!(
            app.conversation[0].tool_calls[0].status,
            ToolCallStatus::Running
        ));
    }

    #[test]
    fn test_tool_result_updates_status() {
        let mut app = App::new();
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
        });
        app.handle_event(TuiEvent::ToolResult {
            id: "tc1".into(),
            result: ToolResult::Success {
                content: "file content".into(),
            },
        });
        assert!(matches!(
            app.conversation[0].tool_calls[0].status,
            ToolCallStatus::Done(_)
        ));
    }

    #[test]
    fn test_done_sets_resting() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Done);
        assert!(matches!(app.state, AppState::Resting));
    }

    #[test]
    fn test_error_sets_resting_and_message() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Error("API error".into()));
        assert!(matches!(app.state, AppState::Resting));
        assert_eq!(app.status_message, "API error");
    }

    #[test]
    fn test_clear_resets_state() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::StreamChunk("Hi".into()));
        app.usage = Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
        });
        app.clear();
        assert!(app.conversation.is_empty());
        assert!(app.input.is_empty());
        assert!(app.usage.is_none());
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_conversation_preserves_history() {
        let mut app = App::new();
        // Simulate two submit/response cycles
        app.handle_event(TuiEvent::Submit("first message".into()));
        app.handle_event(TuiEvent::StreamChunk("first response".into()));
        app.handle_event(TuiEvent::Done);

        app.handle_event(TuiEvent::Submit("second message".into()));
        app.handle_event(TuiEvent::StreamChunk("second response".into()));
        app.handle_event(TuiEvent::Done);

        assert_eq!(app.conversation.len(), 4);
        assert_eq!(app.conversation[0].text, "first message");
        assert_eq!(app.conversation[1].text, "first response");
        assert_eq!(app.conversation[2].text, "second message");
        assert_eq!(app.conversation[3].text, "second response");
    }

    #[test]
    fn test_stream_chunk_without_existing_assistant_creates_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::StreamChunk("direct response".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].text, "direct response");
    }

    #[test]
    fn test_usage_stored() {
        let mut app = App::new();
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
        };
        app.handle_event(TuiEvent::Usage(usage));
        assert_eq!(app.usage.as_ref().unwrap().input_tokens, 100);
        assert_eq!(app.usage.as_ref().unwrap().output_tokens, 50);
    }

    // ── Permission prompt tests ──────────────────────────────────────────

    #[test]
    fn test_permission_prompt_initial_state() {
        let app = App::new();
        assert!(app.pending_permission.is_none());
    }

    #[test]
    fn test_permission_request_stored_in_app() {
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        app.handle_event(TuiEvent::PermissionRequest {
            id: 42,
            reason,
            response: tx,
        });

        assert!(app.pending_permission.is_some());
        let prompt = app.pending_permission.as_ref().unwrap();
        assert_eq!(prompt.id, 42);
        assert_eq!(
            prompt.reason.guard,
            hackpi_guardrails::GuardType::CommandGate
        );
        assert_eq!(prompt.reason.tool, "bash");
        assert!(prompt.response.is_some());
    }

    #[test]
    fn test_pending_permission_cleared_after_decision_sent() {
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let reason = GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        app.handle_event(TuiEvent::PermissionRequest {
            id: 1,
            reason,
            response: tx,
        });

        // Simulate taking the prompt and sending a decision
        if let Some(mut prompt) = app.pending_permission.take() {
            if let Some(sender) = prompt.response.take() {
                sender.send(PermissionDecision::Deny).ok();
            }
        }

        assert!(app.pending_permission.is_none());
        assert_eq!(rx.try_recv(), Ok(PermissionDecision::Deny));
    }

    // ── Permission decision key mapping tests ────────────────────────────

    #[test]
    fn test_key_1_maps_to_allow_once() {
        assert_eq!(
            permission_decision_from_key('1'),
            Some(PermissionDecision::AllowOnce)
        );
    }

    #[test]
    fn test_key_2_maps_to_allow_session() {
        assert_eq!(
            permission_decision_from_key('2'),
            Some(PermissionDecision::AllowSession)
        );
    }

    #[test]
    fn test_key_3_maps_to_deny() {
        assert_eq!(
            permission_decision_from_key('3'),
            Some(PermissionDecision::Deny)
        );
    }

    #[test]
    fn test_key_4_maps_to_always_allow() {
        assert_eq!(
            permission_decision_from_key('4'),
            Some(PermissionDecision::AlwaysAllow)
        );
    }

    #[test]
    fn test_key_5_maps_to_always_deny() {
        assert_eq!(
            permission_decision_from_key('5'),
            Some(PermissionDecision::AlwaysDeny)
        );
    }

    #[test]
    fn test_other_keys_return_none() {
        assert_eq!(permission_decision_from_key('0'), None);
        assert_eq!(permission_decision_from_key('6'), None);
        assert_eq!(permission_decision_from_key('a'), None);
        assert_eq!(permission_decision_from_key(' '), None);
    }
}
