use crate::app::{handle_slash_command, App, CommandOutcome};
use crate::events::TuiEvent;
use crate::input::InputHandler;
use crate::script::{
    action_label, assertions::run_assertions, loader::load_scenario, parse_key, Action,
};
use crate::ui;
use crossterm::event::{KeyCode, KeyModifiers};
use hackpi_core::tools::ToolRegistry;
use hackpi_guardrails::{GuardEvaluator, SettingsPaths};
use ratatui::backend::TestBackend;
use std::path::Path;
use std::sync::{Arc, RwLock};

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
    let scenario = load_scenario(path).await?;

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
        let act = step.action.clone();
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
        println!("  [{step_num}/{total}] {} — PASS", action_label(&act));
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
    if app.pending_permission.is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::UiStatus;
    use crate::events::TuiEvent;

    #[test]
    fn test_handle_key_ctrl_c_interrupts_when_generating() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        app.ui_status = UiStatus::Generating;
        handle_key_in_app(&mut app, &mut input, "CtrlC");
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
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].text, "hi");
        assert_eq!(app.ui_status, UiStatus::Generating);
    }

    #[test]
    fn test_handle_key_enter_noop_with_empty_buffer() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "Enter");
        assert!(app.conversation.is_empty());
    }

    #[test]
    fn test_handle_key_unknown_key_is_noop() {
        let mut app = App::new();
        let mut input = InputHandler::new();
        handle_key_in_app(&mut app, &mut input, "NonExistent");
        assert!(app.conversation.is_empty());
        assert!(input.buffer.is_empty());
    }

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
        // Esc should deny — handle_key_in_app routes Esc to Deny for permission prompts.
        handle_key_in_app(&mut app, &mut input, "Esc");
        assert!(app.pending_permission.is_none());
        assert_eq!(
            rx.try_recv(),
            Ok(hackpi_guardrails::PermissionDecision::Deny)
        );
    }
}
