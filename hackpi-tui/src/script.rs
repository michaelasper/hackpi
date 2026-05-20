use crate::app::{handle_slash_command, App, CommandOutcome, UiStatus};
use crate::events::TuiEvent;
use crate::input::InputHandler;
use crate::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use hackpi_core::tools::ToolRegistry;
use hackpi_guardrails::{GuardEvaluator, SettingsPaths};
use ratatui::backend::TestBackend;
use serde::Deserialize;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// A complete test scenario loaded from a JSON file.
#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<ScenarioStep>,
}

/// A single step in a test scenario.
#[derive(Debug, Deserialize)]
pub struct ScenarioStep {
    /// The action to perform, flattened so its tag field comes from the same
    /// JSON level as `assert`.
    #[serde(flatten)]
    pub action: Action,
    /// Optional assertions to run after processing the action.
    pub assert: Option<Assertions>,
}

/// The action to perform in a scenario step.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Submit text (as if typed and Enter pressed).
    Submit { text: String },
    /// Simulate a single key press.
    Key { key: String },
    /// Wait for a duration in milliseconds.
    Wait { ms: u64 },
}

/// Assertions that must hold after a step's action is processed.
#[derive(Debug, Deserialize)]
pub struct Assertions {
    /// Expected number of conversation entries.
    pub conversation_len: Option<usize>,
    /// Rendered output must contain this substring.
    pub render_contains: Option<String>,
    /// Rendered output must NOT contain this substring.
    pub not_render_contains: Option<String>,
    /// Expected AppState name ("Resting", "Generating", "Interrupted").
    pub state: Option<String>,
    /// Expected status_message content.
    pub status_message: Option<String>,
    /// Files that must exist after this step.
    pub files_exist: Option<Vec<String>>,
}

/// Run a JSON scenario file through the TUI state machine.
///
/// Reads the file, creates an in-memory `App` + `TestBackend` terminal,
/// executes each step sequentially, renders after each step, and runs
/// assertions. Returns `Ok(())` if all steps pass, or an error describing
/// the first failure.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be read or parsed.
/// - A step's assertions fail.
/// - A file expected by `files_exist` does not exist.
pub async fn run_scenario(path: &Path) -> anyhow::Result<()> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to read scenario file '{}': {e}", path.display()))?;

    let scenario: Scenario = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse scenario '{}': {e}", path.display()))?;

    tracing::info!(name = %scenario.name, steps = %scenario.steps.len(), "Running scenario");

    let backend = TestBackend::new(120, 40);
    let mut terminal = ratatui::Terminal::new(backend)
        .map_err(|e| anyhow::anyhow!("Failed to create test terminal: {e}"))?;

    let mut app = App::new();
    let mut input = InputHandler::new();

    // Set up minimal context for slash commands.
    let workspace_root = std::env::current_dir()?;
    let settings_paths = SettingsPaths::new(&workspace_root);
    let guard_evaluator = Arc::new(RwLock::new(GuardEvaluator::new(
        true,
        settings_paths.clone(),
    )));
    // Load rules silently (may fail — that's fine for script mode).
    let _ = guard_evaluator.write().unwrap().load_rules();

    let tool_registry = ToolRegistry::new();

    let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();

    let total = scenario.steps.len();
    for (i, step) in scenario.steps.into_iter().enumerate() {
        let step_num = i + 1;

        // 1. Process the action.
        // Clone the action so we still have it for the label after the match.
        let action = step.action.clone();
        match step.action {
            Action::Submit { text } => {
                input.set_submit(text.clone());
                if let Some(submitted) = input.last_submitted() {
                    if submitted.starts_with('/') {
                        let outcome = handle_slash_command(
                            &submitted,
                            &mut app,
                            &tui_tx,
                            &guard_evaluator,
                            &tool_registry,
                        )
                        .await;
                        if outcome == CommandOutcome::ExitRequested {
                            // Allow /quit to exit without error
                            return Ok(());
                        }
                        // Drain TuiEvent channel and process events through app.
                        drain_tui_events(&mut app, &mut tui_rx).await;
                    } else {
                        app.handle_event(TuiEvent::Submit(submitted.clone()));
                    }
                }
            }
            Action::Key { key } => {
                handle_key_in_app(&mut app, &mut input, &key);
            }
            Action::Wait { ms } => {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
        }

        // 2. Render to the test backend.
        terminal
            .draw(|f| ui::render(f, &app))
            .map_err(|e| anyhow::anyhow!("Step {step_num}/{total}: render failed: {e}"))?;

        // 3. Run assertions.
        if let Some(ref assert) = step.assert {
            run_assertions(assert, &app, &terminal, step_num, total)?;
        }

        // 4. Print pass/fail per step to stdout.
        println!("  [{step_num}/{total}] {} — PASS", action_label(&action));
    }

    println!(
        "Scenario '{name}' completed: {total}/{total} steps passed.",
        name = scenario.name
    );
    Ok(())
}

/// Drain pending `TuiEvent` values from the channel and feed them through
/// `app.handle_event()`. This is necessary after calling
/// `handle_slash_command`, which sends its output as channel events.
async fn drain_tui_events(app: &mut App, rx: &mut tokio::sync::mpsc::UnboundedReceiver<TuiEvent>) {
    while let Ok(event) = rx.try_recv() {
        app.handle_event(event);
    }
}

/// Process a simulated key press through the application's key-handling logic.
///
/// This replicates the relevant key-routing from `main.rs` (global controls,
/// autocomplete, task creation, and default navigation) so that scripted
/// scenarios can exercise the same code paths as interactive use.
fn handle_key_in_app(app: &mut App, input: &mut InputHandler, key_str: &str) {
    let key_event = match parse_key(key_str) {
        Some(ke) => ke,
        None => {
            tracing::warn!("Unknown key string '{key_str}', ignoring");
            return;
        }
    };
    // Permission prompt handling
    if app.pending_permission.is_some() {
        // Check Esc first (not a Char).
        if key_event.code == KeyCode::Esc {
            if let Some(mut prompt) = app.pending_permission.take() {
                if let Some(sender) = prompt.response.take() {
                    let _ = sender.send(hackpi_guardrails::PermissionDecision::Deny);
                }
            }
            return;
        }
        // Check numbered permission decisions.
        if let KeyCode::Char(c) = key_event.code {
            let decision = crate::app::permission_decision_from_key(c);
            if let Some(decision) = decision {
                if let Some(mut prompt) = app.pending_permission.take() {
                    if let Some(sender) = prompt.response.take() {
                        let _ = sender.send(decision);
                    }
                }
                return;
            }
        }
        return;
    }

    // Global controls (checked BEFORE modal branches)
    match key_event.code {
        KeyCode::Char('c') if key_event.modifiers == KeyModifiers::CONTROL => {
            if app.ui_status.is_generating() {
                app.set_interrupted();
            }
            return;
        }
        KeyCode::Char('l') if key_event.modifiers == KeyModifiers::CONTROL => {
            app.clear();
            return;
        }
        KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => {
            app.quit_requested = true;
            return;
        }
        _ => {}
    }

    // Autocomplete popover handling
    if app.autocomplete_visible {
        match key_event.code {
            KeyCode::Up => {
                app.autocomplete_prev();
            }
            KeyCode::Down => {
                app.autocomplete_next();
            }
            KeyCode::Tab => {
                if let Some(cmd) = app.autocomplete_select() {
                    let full_cmd = format!("{} ", cmd);
                    input.buffer = full_cmd;
                    input.cursor = input.buffer.len();
                    app.autocomplete_visible = false;
                }
            }
            KeyCode::Enter => {
                if let Some(cmd) = app.autocomplete_select() {
                    let full_cmd = cmd.to_string();
                    input.set_submit(full_cmd);
                    app.autocomplete_visible = false;
                } else {
                    input.handle_key(key_event);
                }
            }
            KeyCode::Esc => {
                app.autocomplete_visible = false;
            }
            _ => {
                input.handle_key(key_event);
            }
        }
    } else if app.creating_task {
        // Task creation inline prompt
        match key_event.code {
            KeyCode::Enter => {
                app.submit_create_task();
            }
            KeyCode::Esc => {
                app.cancel_create_task();
            }
            KeyCode::Backspace => {
                app.task_create_input.pop();
            }
            KeyCode::Char(ch) => {
                app.task_create_input.push(ch);
            }
            _ => {}
        }
    } else {
        // Default key handling
        match key_event.code {
            KeyCode::Tab => {
                app.cycle_view();
                if matches!(app.active_view, crate::app::AppView::TaskBoard) {
                    app.refresh_task_cache();
                }
            }
            KeyCode::Up => {
                if matches!(app.active_view, crate::app::AppView::TaskBoard) {
                    app.task_cursor_up();
                } else if matches!(app.active_view, crate::app::AppView::TaskDetail(_)) {
                    app.task_detail_prev();
                } else {
                    app.auto_scroll = false;
                    app.scroll_offset = app.scroll_offset.saturating_sub(3);
                }
            }
            KeyCode::Down => {
                if matches!(app.active_view, crate::app::AppView::TaskBoard) {
                    app.task_cursor_down();
                } else if matches!(app.active_view, crate::app::AppView::TaskDetail(_)) {
                    app.task_detail_next();
                } else {
                    app.auto_scroll = false;
                    app.scroll_offset = app.scroll_offset.saturating_add(3);
                }
            }
            KeyCode::Enter => {
                let has_input = !input.buffer.trim().is_empty();
                if matches!(app.active_view, crate::app::AppView::TaskBoard) && !has_input {
                    app.enter_task_detail();
                } else if !app.ui_status.is_active() {
                    input.handle_key(key_event);
                }
            }
            KeyCode::Esc => {
                if !matches!(app.active_view, crate::app::AppView::Conversation) {
                    app.go_back();
                    if matches!(app.active_view, crate::app::AppView::TaskBoard) {
                        app.refresh_task_cache();
                    }
                }
            }
            KeyCode::Char('n') => {
                if matches!(
                    app.active_view,
                    crate::app::AppView::TaskBoard | crate::app::AppView::TaskDetail(_)
                ) {
                    app.begin_create_task();
                } else {
                    input.handle_key(key_event);
                }
            }
            KeyCode::PageUp => {
                if matches!(app.active_view, crate::app::AppView::Conversation) {
                    app.auto_scroll = false;
                }
                app.scroll_offset = app.scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                if matches!(app.active_view, crate::app::AppView::Conversation) {
                    app.auto_scroll = false;
                }
                app.scroll_offset = app.scroll_offset.saturating_add(10);
            }
            KeyCode::Home => {
                if matches!(app.active_view, crate::app::AppView::Conversation) {
                    app.auto_scroll = false;
                }
                app.scroll_offset = 0;
            }
            KeyCode::End => {
                if matches!(app.active_view, crate::app::AppView::Conversation) {
                    app.auto_scroll = true;
                }
            }
            _ => {
                if !app.ui_status.is_active() {
                    input.handle_key(key_event);
                }
            }
        }
    }

    // Sync input buffer to app.input for display after every key event.
    app.input = input.buffer.clone();
    app.input_cursor = input.cursor;
    app.update_autocomplete_state();

    // Process any submitted text (from Enter handling).
    if !app.ui_status.is_active() {
        if let Some(submitted) = input.last_submitted() {
            if submitted.starts_with('/') {
                // Can't handle slash commands here synchronously, so just
                // push as a user conversation entry as a best-effort fallback.
                app.handle_event(TuiEvent::Submit(submitted));
            } else {
                app.handle_event(TuiEvent::Submit(submitted));
            }
        }
    }
}

/// Run assertions against the current app state and rendered buffer.
fn run_assertions(
    assert: &Assertions,
    app: &App,
    terminal: &ratatui::Terminal<TestBackend>,
    step_num: usize,
    total: usize,
) -> anyhow::Result<()> {
    let buffer = terminal.backend().buffer();
    let cell_str: String = buffer
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<Vec<&str>>()
        .concat();

    if let Some(expected_len) = assert.conversation_len {
        let actual_len = app.conversation.len();
        if actual_len != expected_len {
            anyhow::bail!(
                "Step {step_num}/{total}: expected conversation_len={expected_len}, got {actual_len}"
            );
        }
    }

    if let Some(ref expected) = assert.render_contains {
        if !cell_str.contains(expected.as_str()) {
            anyhow::bail!(
                "Step {step_num}/{total}: expected render to contain '{expected}', \
                 but it was not found.\nFull render:\n{cell_str}"
            );
        }
    }

    if let Some(ref unexpected) = assert.not_render_contains {
        if cell_str.contains(unexpected.as_str()) {
            anyhow::bail!(
                "Step {step_num}/{total}: expected render to NOT contain '{unexpected}', \
                 but it was found.\nFull render:\n{cell_str}"
            );
        }
    }

    if let Some(ref expected_state) = assert.state {
        let actual_state = match app.ui_status {
            UiStatus::Idle => "Idle",
            UiStatus::Generating => "Generating",
            UiStatus::RunningTool { .. } => "RunningTool",
            UiStatus::LoadingTasks => "LoadingTasks",
            UiStatus::WaitingForPermission => "WaitingForPermission",
            UiStatus::Error { .. } => "Error",
        };
        let expected_legacy = match expected_state.as_str() {
            "Resting" => "Idle",
            "Interrupted" => "Idle",
            other => other,
        };
        if actual_state != expected_legacy {
            anyhow::bail!(
                "Step {step_num}/{total}: expected state='{expected_state}', got '{actual_state}'"
            );
        }
    }

    if let Some(ref expected_msg) = assert.status_message {
        match &app.info_message {
            Some(actual_msg) if actual_msg == expected_msg => {}
            None if expected_msg.is_empty() => {}
            _ => {
                anyhow::bail!(
                    "Step {step_num}/{total}: expected status_message='{expected_msg}', \
                     got '{:?}'",
                    app.info_message
                );
            }
        }
    }

    if let Some(ref files) = assert.files_exist {
        for file_path in files {
            if !Path::new(file_path).exists() {
                anyhow::bail!(
                    "Step {step_num}/{total}: expected file '{file_path}' to exist, \
                     but it was not found"
                );
            }
        }
    }

    Ok(())
}

/// Parse a key identifier string into a `crossterm::event::KeyEvent`.
///
/// Supported values:
/// - `Up`, `Down`, `Enter`, `Esc`, `Tab`, `Backspace`, `Delete`
/// - `PageUp`, `PageDown`, `Home`, `End`
/// - `CtrlC`, `CtrlD`, `CtrlL`
/// - `Char(c)` where `c` is a single character (e.g. `Char(n)`)
fn parse_key(s: &str) -> Option<KeyEvent> {
    match s {
        "Up" => Some(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        "Down" => Some(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        "Enter" => Some(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        "Esc" => Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        "Tab" => Some(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        "Backspace" => Some(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        "Delete" => Some(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        "PageUp" => Some(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
        "PageDown" => Some(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
        "Home" => Some(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        "End" => Some(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        "CtrlC" => Some(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        "CtrlD" => Some(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        "CtrlL" => Some(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL)),
        _ => {
            // Try to parse "Char(x)" pattern
            let s_trimmed = s.trim();
            if let Some(inner) = s_trimmed
                .strip_prefix("Char(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                let ch = inner.chars().next()?;
                Some(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
            } else {
                None
            }
        }
    }
}

/// Return a short human-readable label for an action (for per-step output).
fn action_label(action: &Action) -> String {
    match action {
        Action::Submit { text } => {
            if text.len() > 40 {
                format!("submit \"{}...\"", &text[..37])
            } else {
                format!("submit \"{text}\"")
            }
        }
        Action::Key { key } => format!("key [{key}]"),
        Action::Wait { ms } => format!("wait {ms}ms"),
    }
}

/// Parse command-line arguments for `--script`.
///
/// Returns `Some(path)` if `--script <path>` is found, removing those
/// arguments from the list. Returns `None` if `--script` is not present.
pub fn parse_script_args(args: &mut Vec<String>) -> Option<String> {
    let script_flag_pos = args.iter().position(|a| a == "--script")?;
    // Remove the flag first.
    args.remove(script_flag_pos);
    // The value that was after the flag is now at `script_flag_pos`.
    let script_path = args.get(script_flag_pos)?;
    let path = script_path.clone();
    args.remove(script_flag_pos);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Key parsing tests ───────────────────────────────────────────────

    #[test]
    fn test_parse_key_navigation() {
        assert_eq!(parse_key("Up").unwrap().code, KeyCode::Up,);
        assert_eq!(parse_key("Down").unwrap().code, KeyCode::Down,);
        assert_eq!(parse_key("Enter").unwrap().code, KeyCode::Enter,);
        assert_eq!(parse_key("Esc").unwrap().code, KeyCode::Esc,);
        assert_eq!(parse_key("Tab").unwrap().code, KeyCode::Tab,);
        assert_eq!(parse_key("PageUp").unwrap().code, KeyCode::PageUp,);
        assert_eq!(parse_key("PageDown").unwrap().code, KeyCode::PageDown,);
    }

    #[test]
    fn test_parse_key_control() {
        let ctrl_c = parse_key("CtrlC").unwrap();
        assert_eq!(ctrl_c.code, KeyCode::Char('c'));
        assert_eq!(ctrl_c.modifiers, KeyModifiers::CONTROL);

        let ctrl_d = parse_key("CtrlD").unwrap();
        assert_eq!(ctrl_d.code, KeyCode::Char('d'));
        assert_eq!(ctrl_d.modifiers, KeyModifiers::CONTROL);

        let ctrl_l = parse_key("CtrlL").unwrap();
        assert_eq!(ctrl_l.code, KeyCode::Char('l'));
        assert_eq!(ctrl_l.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn test_parse_key_char() {
        let char_n = parse_key("Char(n)").unwrap();
        assert_eq!(char_n.code, KeyCode::Char('n'));
        assert_eq!(char_n.modifiers, KeyModifiers::NONE);

        let char_a = parse_key("Char(a)").unwrap();
        assert_eq!(char_a.code, KeyCode::Char('a'));

        let char_slash = parse_key("Char(/)").unwrap();
        assert_eq!(char_slash.code, KeyCode::Char('/'));
    }

    #[test]
    fn test_parse_key_unknown_returns_none() {
        assert!(parse_key("NonExistent").is_none());
        assert!(parse_key("").is_none());
        assert!(parse_key("Char()").is_none());
    }

    // ── Action label tests ──────────────────────────────────────────────

    #[test]
    fn test_action_label_submit_short() {
        let label = action_label(&Action::Submit {
            text: "hello".into(),
        });
        assert_eq!(label, "submit \"hello\"");
    }

    #[test]
    fn test_action_label_submit_long_truncated() {
        let long = "a".repeat(50);
        let label = action_label(&Action::Submit { text: long });
        assert!(label.ends_with("...\""));
        assert!(label.len() < 50 + 20, "label should be truncated: {label}");
    }

    #[test]
    fn test_action_label_key() {
        let label = action_label(&Action::Key {
            key: "Enter".into(),
        });
        assert_eq!(label, "key [Enter]");
    }

    #[test]
    fn test_action_label_wait() {
        let label = action_label(&Action::Wait { ms: 500 });
        assert_eq!(label, "wait 500ms");
    }

    // ── Scenario JSON parsing tests ─────────────────────────────────────

    #[test]
    fn test_parse_submit_scenario() {
        let json = r#"{
            "name": "Test Submit",
            "steps": [
                {
                    "action": "submit",
                    "text": "/help",
                    "assert": {
                        "conversation_len": 0,
                        "render_contains": "Available commands"
                    }
                }
            ]
        }"#;
        let scenario: Scenario = serde_json::from_str(json).expect("parse scenario");
        assert_eq!(scenario.name, "Test Submit");
        assert_eq!(scenario.steps.len(), 1);
        match &scenario.steps[0].action {
            Action::Submit { text } => assert_eq!(text, "/help"),
            _ => panic!("expected Submit action"),
        }
        let assert = scenario.steps[0].assert.as_ref().expect("assertions");
        assert_eq!(assert.conversation_len, Some(0));
        assert_eq!(assert.render_contains, Some("Available commands".into()));
    }

    #[test]
    fn test_parse_key_scenario() {
        let json = r#"{
            "name": "Key Test",
            "steps": [
                {
                    "action": "key",
                    "key": "Enter"
                },
                {
                    "action": "key",
                    "key": "Tab",
                    "assert": {
                        "state": "Resting"
                    }
                }
            ]
        }"#;
        let scenario: Scenario = serde_json::from_str(json).expect("parse scenario");
        assert_eq!(scenario.steps.len(), 2);
        match &scenario.steps[0].action {
            Action::Key { key } => assert_eq!(key, "Enter"),
            _ => panic!("expected Key action"),
        }
        assert!(scenario.steps[0].assert.is_none());
        assert!(scenario.steps[1].assert.is_some());
    }

    #[test]
    fn test_parse_wait_scenario() {
        let json = r#"{
            "name": "Wait",
            "steps": [
                {
                    "action": "wait",
                    "ms": 100
                }
            ]
        }"#;
        let scenario: Scenario = serde_json::from_str(json).expect("parse scenario");
        match &scenario.steps[0].action {
            Action::Wait { ms } => assert_eq!(*ms, 100),
            _ => panic!("expected Wait action"),
        }
    }

    #[test]
    fn test_parse_assertions_all_fields() {
        let json = r#"{
            "action": "submit",
            "text": "hello",
            "assert": {
                "conversation_len": 1,
                "render_contains": "hello",
                "not_render_contains": "goodbye",
                "state": "Generating",
                "status_message": "",
                "files_exist": ["/tmp/test.txt"]
            }
        }"#;
        let step: ScenarioStep = serde_json::from_str(json).expect("parse step");
        let assert = step.assert.expect("assertions");
        assert_eq!(assert.conversation_len, Some(1));
        assert_eq!(assert.render_contains, Some("hello".into()));
        assert_eq!(assert.not_render_contains, Some("goodbye".into()));
        assert_eq!(assert.state, Some("Generating".into()));
        assert_eq!(assert.status_message, Some("".into()));
        assert_eq!(assert.files_exist, Some(vec!["/tmp/test.txt".into()]));
    }

    // ── parse_script_args tests ────────────────────────────────────────

    #[test]
    fn test_parse_script_args_found() {
        let mut args: Vec<String> = vec![
            "hackpi".to_string(),
            "--script".to_string(),
            "scenario.json".to_string(),
            "--god".to_string(),
        ];
        let path = parse_script_args(&mut args);
        assert_eq!(path, Some("scenario.json".to_string()));
        assert_eq!(args, vec!["hackpi".to_string(), "--god".to_string()]);
    }

    #[test]
    fn test_parse_script_args_not_found() {
        let mut args: Vec<String> = vec!["hackpi".to_string(), "--god".to_string()];
        let path = parse_script_args(&mut args);
        assert!(path.is_none());
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn test_parse_script_args_only_flag_no_value() {
        // If --script is last arg, there's no value following.
        let mut args: Vec<String> = vec!["hackpi".to_string(), "--script".to_string()];
        let path = parse_script_args(&mut args);
        assert!(path.is_none());
        // The flag should still be removed since we found it
        assert_eq!(args, vec!["hackpi".to_string()]);
    }

    // ── handle_key_in_app tests ─────────────────────────────────────────

    #[test]
    fn test_handle_key_ctrl_c_interrupts_when_generating() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        app.ui_status = UiStatus::Generating;
        handle_key_in_app(&mut app, &mut input, "CtrlC");
        // Interrupted sets Idle with an info message
        assert_eq!(app.ui_status, UiStatus::Idle);
        assert_eq!(app.info_message, Some("Generation interrupted.".into()));
    }

    #[test]
    fn test_handle_key_ctrl_c_noop_when_idle() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        app.ui_status = UiStatus::Idle;
        handle_key_in_app(&mut app, &mut input, "CtrlC");
        assert_eq!(app.ui_status, UiStatus::Idle);
    }

    #[test]
    fn test_handle_key_ctrl_l_clears() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        handle_key_in_app(&mut app, &mut input, "CtrlL");
        assert!(
            app.conversation.is_empty(),
            "Ctrl+L should clear conversation"
        );
    }

    #[test]
    fn test_handle_key_ctrl_d_requests_quit() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        assert!(!app.quit_requested);
        handle_key_in_app(&mut app, &mut input, "CtrlD");
        assert!(app.quit_requested);
    }

    #[test]
    fn test_handle_key_tab_cycles_view() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        assert!(matches!(app.active_view, crate::app::AppView::Conversation));
        handle_key_in_app(&mut app, &mut input, "Tab");
        assert!(matches!(app.active_view, crate::app::AppView::TaskBoard));
    }

    #[test]
    fn test_handle_key_char_types_text() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "Char(h)");
        handle_key_in_app(&mut app, &mut input, "Char(e)");
        handle_key_in_app(&mut app, &mut input, "Char(l)");
        handle_key_in_app(&mut app, &mut input, "Char(l)");
        handle_key_in_app(&mut app, &mut input, "Char(o)");
        assert_eq!(input.buffer, "hello");
        assert_eq!(app.input, "hello");
    }

    #[test]
    fn test_handle_key_char_blocked_when_generating() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        app.ui_status = UiStatus::Generating;
        handle_key_in_app(&mut app, &mut input, "Char(x)");
        assert!(
            input.buffer.is_empty(),
            "typing should be blocked during Generating"
        );
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "Char(a)");
        handle_key_in_app(&mut app, &mut input, "Char(b)");
        assert_eq!(input.buffer, "ab");
        handle_key_in_app(&mut app, &mut input, "Backspace");
        assert_eq!(input.buffer, "a");
    }

    #[test]
    fn test_handle_key_enter_submits_text() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "Char(h)");
        handle_key_in_app(&mut app, &mut input, "Char(i)");
        handle_key_in_app(&mut app, &mut input, "Enter");
        // After Enter + submit handling, conversation should have a user entry
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].text, "hi");
        assert_eq!(app.ui_status, UiStatus::Generating);
    }

    #[test]
    fn test_handle_key_enter_noop_with_empty_buffer() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "Enter");
        // Empty buffer should not submit - conversation stays empty
        assert!(app.conversation.is_empty());
    }

    #[test]
    fn test_handle_key_unknown_key_is_noop() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        // non-existent key string should not panic
        handle_key_in_app(&mut app, &mut input, "NonExistent");
        assert!(app.conversation.is_empty());
        assert!(input.buffer.is_empty());
    }

    // ── Permission decision via key tests ───────────────────────────────

    #[test]
    fn test_handle_key_permission_decision_1_allow_once() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "test".into(),
            },
            response: Some(tx),
            confirming_always_allow: false,
        });
        handle_key_in_app(&mut app, &mut input, "Char(1)");
        assert!(app.pending_permission.is_none());
        assert_eq!(
            rx.try_recv(),
            Ok(hackpi_guardrails::PermissionDecision::AllowOnce)
        );
    }

    #[test]
    fn test_handle_key_permission_esc_denies() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "test".into(),
            },
            response: Some(tx),
            confirming_always_allow: false,
        });
        // Esc should deny
        // Permission prompt handling in handle_key_in_app checks for Esc via
        // the KeyCode match. Since parse_key("Esc") returns Esc with NONE modifiers,
        // and the permission handler checks key_event.code == KeyCode::Esc,
        // this should work.
        handle_key_in_app(&mut app, &mut input, "Esc");
        assert!(app.pending_permission.is_none());
        assert_eq!(
            rx.try_recv(),
            Ok(hackpi_guardrails::PermissionDecision::Deny)
        );
    }

    // ── Scenario runner integration test ─────────────────────────────────

    #[tokio::test]
    async fn test_run_help_and_clear_scenario() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scenario_path = dir.path().join("test-help-and-clear.json");
        let scenario_json = r#"{
            "name": "Help and Clear",
            "steps": [
                {
                    "action": "submit",
                    "text": "/help",
                    "assert": {
                        "render_contains": "Available commands",
                        "state": "Resting"
                    }
                },
                {
                    "action": "submit",
                    "text": "/clear",
                    "assert": {
                        "conversation_len": 0,
                        "state": "Resting"
                    }
                }
            ]
        }"#;
        std::fs::write(&scenario_path, scenario_json).expect("write scenario");

        let result = run_scenario(&scenario_path).await;
        assert!(result.is_ok(), "scenario should pass: {:?}", result.err());
    }

    #[tokio::test]
    async fn test_run_submit_and_check_scenario() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scenario_path = dir.path().join("test-submit.json");
        let scenario_json = r#"{
            "name": "Submit Text",
            "steps": [
                {
                    "action": "submit",
                    "text": "Hello, world!",
                    "assert": {
                        "conversation_len": 1,
                        "render_contains": "Hello, world!",
                        "state": "Generating"
                    }
                }
            ]
        }"#;
        std::fs::write(&scenario_path, scenario_json).expect("write scenario");

        let result = run_scenario(&scenario_path).await;
        assert!(result.is_ok(), "scenario should pass: {:?}", result.err());
    }

    #[tokio::test]
    async fn test_run_scenario_fails_on_missing_file() {
        let result = run_scenario(Path::new("/nonexistent/scenario.json")).await;
        assert!(result.is_err(), "should fail on missing file");
        let err = result.err().unwrap();
        let msg = format!("{err}");
        assert!(msg.contains("Failed to read scenario"), "error: {msg}");
    }

    #[tokio::test]
    async fn test_run_scenario_fails_on_bad_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bad_path = dir.path().join("bad.json");
        std::fs::write(&bad_path, "not valid json}{").expect("write bad json");
        let result = run_scenario(&bad_path).await;
        assert!(result.is_err(), "should fail on bad JSON");
        let err = result.err().unwrap();
        let msg = format!("{err}");
        assert!(msg.contains("Failed to parse scenario"), "error: {msg}");
    }

    #[tokio::test]
    async fn test_run_scenario_assertion_failure_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scenario_path = dir.path().join("fail-scenario.json");
        let scenario_json = r#"{
            "name": "Should Fail",
            "steps": [
                {
                    "action": "submit",
                    "text": "hello",
                    "assert": {
                        "conversation_len": 99,
                        "render_contains": "NONEXISTENT"
                    }
                }
            ]
        }"#;
        std::fs::write(&scenario_path, scenario_json).expect("write scenario");

        let result = run_scenario(&scenario_path).await;
        assert!(result.is_err(), "should fail on bad assertion");
        let err = result.err().unwrap();
        let msg = format!("{err}");
        assert!(
            msg.contains("conversation_len"),
            "error should mention conversation_len, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_run_scenario_with_key_actions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let scenario_path = dir.path().join("key-scenario.json");
        let scenario_json = r#"{
            "name": "Key Actions",
            "steps": [
                {
                    "action": "key",
                    "key": "Char(h)",
                    "assert": {
                        "render_contains": "> h",
                        "state": "Resting"
                    }
                },
                {
                    "action": "key",
                    "key": "Char(i)",
                    "assert": {
                        "render_contains": "> hi"
                    }
                },
                {
                    "action": "key",
                    "key": "Enter",
                    "assert": {
                        "conversation_len": 1,
                        "render_contains": "○ me: hi",
                        "state": "Generating"
                    }
                }
            ]
        }"#;
        std::fs::write(&scenario_path, scenario_json).expect("write scenario");

        let result = run_scenario(&scenario_path).await;
        assert!(result.is_ok(), "scenario should pass: {:?}", result.err());
    }
}
