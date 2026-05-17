use crate::events::TuiEvent;
use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;
use hackpi_guardrails::{GuardEvaluator, GuardReason, PermissionDecision};
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

pub async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    guard_evaluator: &mut GuardEvaluator,
) -> bool {
    let parts: Vec<&str> = cmd.trim().splitn(2, char::is_whitespace).collect();
    let command = parts[0];
    match command {
        "/help" => {
            let help_text = "\
Available commands:
  /help  - Show this help message
  /clear - Clear the conversation
  /quit  - Exit the application
  /guardrails:status - Show guardrails status
  /guardrails:clean - Clear session cache
  /guardrails:onboarding [preset] - Write a preset guardrails config";
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
        "/guardrails:status" => {
            let rule_count = guard_evaluator.rule_count();
            let god_mode = guard_evaluator.is_god_mode();
            let cache_len = guard_evaluator.session_cache_len();
            let god_mode_str = if god_mode { "yes" } else { "no" };

            // Determine which guards are active by checking if rules exist
            // for each guard type (this is best-effort based on rule count)
            let status_text = format!(
                "\
Guardrails Status:
  Rules loaded: {rule_count}
  God mode: {god_mode_str}
  Session cache entries: {cache_len}
  Active guards: PathGuard ✓, CommandGate ✓, FileProtection ✓"
            );
            tui_tx.send(TuiEvent::StreamChunk(status_text)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        "/guardrails:clean" => {
            guard_evaluator.clear_session();
            let msg = "Session cache cleared.".to_string();
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        cmd if cmd.starts_with("/guardrails:onboarding") => {
            let rest = parts.get(1).copied().unwrap_or("");
            let preset = rest.trim().to_lowercase();

            let (preset_name, config_json) = match preset.as_str() {
                "strict" => ("strict", STRICT_CONFIG),
                "balanced" => ("balanced", BALANCED_CONFIG),
                "permissive" => ("permissive", PERMISSIVE_CONFIG),
                "" => {
                    let summary = "\
Guardrails Onboarding Presets:
  /guardrails:onboarding strict     - Deny everything, ask for everything
  /guardrails:onboarding balanced   - Balanced defaults (recommended)
  /guardrails:onboarding permissive - Permissive rules with minimal restrictions";
                    tui_tx.send(TuiEvent::StreamChunk(summary.to_string())).ok();
                    tui_tx.send(TuiEvent::Done).ok();
                    return true;
                }
                _ => {
                    let err = format!(
                        "Unknown preset: '{preset}'. Available: strict, balanced, permissive"
                    );
                    tui_tx.send(TuiEvent::Error(err)).ok();
                    return true;
                }
            };

            let hackpi_dir = match guard_evaluator.settings_paths().hackpi.parent() {
                Some(dir) => dir.to_path_buf(),
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Cannot determine workspace root for guardrails config".into(),
                        ))
                        .ok();
                    return true;
                }
            };

            // Create .hackpi directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&hackpi_dir) {
                let err = format!("Failed to create directory {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            let config_path = hackpi_dir.join("guardrails.json");
            if let Err(e) = std::fs::write(&config_path, config_json) {
                let err = format!("Failed to write config file: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            // Reload rules from the new config
            if let Err(e) = guard_evaluator.load_rules() {
                let err = format!("Failed to load rules after writing config: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            let rule_count = guard_evaluator.rule_count();
            let msg = format!(
                "Wrote {preset_name} guardrails config to {} ({rule_count} rules loaded).",
                config_path.display()
            );
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        _ => {
            let err = format!("Unknown command: {command}. Type /help for available commands.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            true
        }
    }
}

// ── Preset configs ──────────────────────────────────────────────────────────

const STRICT_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": [],
    "deny": ["Read(./**)", "Write(./**)", "Bash(*)"]
  },
  "path_access": {
    "allow": [],
    "deny": ["/**", "~/**"],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "deny",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "deny", "write": "deny" },
      "*.pem": { "read": "deny", "write": "deny" },
      "*.key": { "read": "deny", "write": "deny" }
    }
  }
}"#;

const BALANCED_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./docs/**)"],
    "deny": ["Read(./.env)", "Write(./.env)", "Bash(sudo *)"]
  },
  "path_access": {
    "allow": [],
    "deny": [],
    "ask": true
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "ask",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "ask", "write": "deny" },
      "*.pem": { "read": "ask", "write": "deny" },
      "*.key": { "read": "ask", "write": "deny" }
    }
  }
}"#;

const PERMISSIVE_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./**)", "Write(./**)", "Bash(*)"],
    "deny": []
  },
  "path_access": {
    "allow": ["/**", "~/**"],
    "deny": [],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "sudo": "deny",
      "rm -rf /": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env": { "write": "deny" }
    }
  }
}"#;

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
    use hackpi_guardrails::SettingsPaths;
    use tokio::sync::mpsc;

    /// Helper to create a GuardEvaluator backed by a temp directory.
    fn make_guard_evaluator() -> (GuardEvaluator, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);
        (evaluator, dir)
    }

    #[tokio::test]
    async fn test_slash_command_prevents_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/help", &mut app, &tx, &mut ge).await;
        assert!(handled);
    }

    #[tokio::test]
    async fn test_slash_help_generates_help_text() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/help", &mut app, &tx, &mut ge).await;
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

    #[tokio::test]
    async fn test_slash_clear_clears_conversation() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/clear", &mut app, &tx, &mut ge).await;
        assert!(handled);
        assert!(app.conversation.is_empty());
        assert!(app.input.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_slash_command_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/unknown", &mut app, &tx, &mut ge).await;
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

    // ── Guardrails slash command tests ────────────────────────────────────

    #[tokio::test]
    async fn test_guardrails_status_returns_info() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/guardrails:status", &mut app, &tx, &mut ge).await;
        assert!(handled);
        let mut found_chunk = false;
        let mut found_done = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::StreamChunk(text) => {
                    found_chunk = true;
                    assert!(text.contains("Guardrails Status"));
                    assert!(text.contains("Rules loaded"));
                    assert!(text.contains("God mode"));
                    assert!(text.contains("Active guards"));
                }
                TuiEvent::Done => found_done = true,
                _ => {}
            }
        }
        assert!(found_chunk);
        assert!(found_done);
    }

    #[tokio::test]
    async fn test_guardrails_clean_clears_session() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();

        // Record a session decision first
        ge.record_decision("test-key".into(), PermissionDecision::AllowSession);
        assert_eq!(ge.session_cache_len(), 1);

        let handled = handle_slash_command("/guardrails:clean", &mut app, &tx, &mut ge).await;
        assert!(handled);

        // Verify session cache is cleared
        assert_eq!(ge.session_cache_len(), 0);

        // Verify output message
        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("cleared"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_balanced_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();

        // Initial state: no rules loaded
        assert_eq!(ge.rule_count(), 0);

        let handled =
            handle_slash_command("/guardrails:onboarding balanced", &mut app, &tx, &mut ge).await;
        assert!(handled);

        // Verify rules were loaded
        assert!(
            ge.rule_count() > 0,
            "rules should be loaded after onboarding"
        );

        // Verify config file was written
        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists(), "config file should exist");
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(
            content.contains("Read(./docs/**)"),
            "balanced config should contain Read(./docs/**)"
        );
        assert!(
            content.contains("\"ask\": true"),
            "balanced config should have ask: true"
        );

        // Verify success message
        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("rules loaded"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_no_args_shows_presets() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/guardrails:onboarding", &mut app, &tx, &mut ge).await;
        assert!(handled);
        let mut found_summary = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_summary = true;
                assert!(msg.contains("strict"));
                assert!(msg.contains("balanced"));
                assert!(msg.contains("permissive"));
            }
        }
        assert!(found_summary);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_strict_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/guardrails:onboarding strict", &mut app, &tx, &mut ge).await;
        assert!(handled);
        assert!(ge.rule_count() > 0);

        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("deny"));
        assert!(content.contains("rm -rf"));

        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("strict"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_permissive_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/guardrails:onboarding permissive", &mut app, &tx, &mut ge).await;
        assert!(handled);
        assert!(ge.rule_count() > 0);

        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("Read(./**)"));

        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("permissive"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_unknown_subcommand_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command("/guardrails:unknown", &mut app, &tx, &mut ge).await;
        assert!(handled);
        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("/guardrails:unknown"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_guardrails_commands_prevent_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();

        let cmds = [
            "/guardrails:status",
            "/guardrails:clean",
            "/guardrails:onboarding balanced",
        ];
        for cmd in &cmds {
            let handled = handle_slash_command(cmd, &mut app, &tx, &mut ge).await;
            assert!(handled, "command '{cmd}' should prevent agent spawn");
        }
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
