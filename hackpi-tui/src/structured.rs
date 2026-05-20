use crate::app::{handle_slash_command, App, CommandOutcome};
use crate::events::TuiEvent;
use hackpi_core::tools::ToolRegistry;
use hackpi_guardrails::GuardEvaluator;
use std::io::{self, BufRead, Write};
use std::sync::{Arc, RwLock};

/// Run hackpi in structured-events mode.
///
/// Reads commands from stdin line by line and outputs machine-readable JSON
/// lines to stdout for each `TuiEvent`. No terminal rendering (ratatui/crossterm)
/// is used. Input is line-based rather than keystroke-based.
///
/// # Errors
///
/// Returns an error if stdin/stdout I/O fails.
pub async fn run_structured_events(
    tool_registry: Arc<ToolRegistry>,
    guard_evaluator: Arc<RwLock<GuardEvaluator>>,
) -> anyhow::Result<()> {
    let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
    let mut app = App::new();
    let stdin = io::stdin();
    let reader = stdin.lock();

    // Spawn a task to process events and write JSON lines to stdout.
    // The StdoutLock is acquired and released within each iteration to avoid
    // holding a non-Send type across an await point.
    let event_handle = tokio::spawn(async move {
        while let Some(event) = tui_rx.recv().await {
            if let Some(json_line) = event.to_json_line() {
                let stdout = io::stdout();
                let mut out = stdout.lock();
                writeln!(out, "{json_line}").ok();
                out.flush().ok();
            }
        }
    });

    // Read input line by line from stdin
    for line in reader.lines() {
        let line = line.map_err(|e| anyhow::anyhow!("Failed to read stdin: {e}"))?;
        tui_tx.send(TuiEvent::Submit(line.clone())).ok();

        if line.starts_with('/') {
            let outcome =
                handle_slash_command(&line, &mut app, &tui_tx, &guard_evaluator, &tool_registry)
                    .await;
            if matches!(outcome, CommandOutcome::ExitRequested) {
                break;
            }
            // Most slash command handlers (e.g., /help, /guardrails:*, /git:*,
            // /task, /tasks, /export) send TuiEvent::Done themselves as part
            // of their response. Only send Done here if the handler did NOT
            // emit one (i.e., NeedsRender, such as /clear).
            if matches!(outcome, CommandOutcome::NeedsRender) {
                tui_tx.send(TuiEvent::Done).ok();
            }
        } else {
            // Non-slash input: no handler was invoked, always send Done.
            tui_tx.send(TuiEvent::Done).ok();
        }
    }

    // Drop the sender so the event task terminates
    drop(tui_tx);
    if let Err(e) = event_handle.await {
        eprintln!("Event writer task error: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_guardrails::SettingsPaths;
    use std::sync::Arc;

    /// Helper: create a GuardEvaluator backed by a temp directory.
    fn make_guard_evaluator() -> (GuardEvaluator, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);
        (evaluator, dir)
    }

    /// Helper: build the full set of dependencies needed for handle_slash_command.
    fn make_deps() -> (
        App,
        Arc<RwLock<GuardEvaluator>>,
        Arc<ToolRegistry>,
        tempfile::TempDir,
    ) {
        let app = App::new();
        let (ge, dir) = make_guard_evaluator();
        let ge = Arc::new(RwLock::new(ge));
        let tool_registry = Arc::new(ToolRegistry::new());
        (app, ge, tool_registry, dir)
    }

    #[tokio::test]
    async fn test_tui_event_emission_through_channel() {
        let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();

        // Simulate what the structured runner does
        tui_tx.send(TuiEvent::Submit("/help".into())).ok();
        tui_tx.send(TuiEvent::Done).ok();
        drop(tui_tx);

        let mut events = Vec::new();
        while let Some(event) = tui_rx.recv().await {
            events.push(event);
        }

        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], TuiEvent::Submit(ref s) if s == "/help"));
        assert!(matches!(events[1], TuiEvent::Done));
    }

    #[tokio::test]
    async fn test_tui_event_channel_produces_json_lines() {
        let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();

        tui_tx.send(TuiEvent::Submit("hello".into())).ok();
        tui_tx.send(TuiEvent::Done).ok();
        drop(tui_tx);

        // Collect and drain
        let mut json_lines = Vec::new();
        while let Some(event) = tui_rx.recv().await {
            if let Some(json) = event.to_json_line() {
                json_lines.push(json);
            }
        }

        assert_eq!(json_lines.len(), 2);
        // Verify both lines are valid JSON
        for line in &json_lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("should be valid JSON");
            assert!(parsed["timestamp"].is_string());
        }
    }

    #[test]
    fn test_guard_evaluator_creation() {
        let (_ge, _dir) = make_guard_evaluator();
        // Just verify it doesn't panic
    }

    // ── Double-done prevention tests ────────────────────────────────────

    #[tokio::test]
    async fn test_handled_commands_emit_own_done() {
        // Handlers like /help, /guardrails:*, /git:*, /task, /export all
        // emit their own Done event internally. The structured loop must
        // NOT add a second Done on top.
        let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        let (mut app, ge, tool_registry, _dir) = make_deps();

        // /help handler sends its own events including Done
        handle_slash_command("/help", &mut app, &tui_tx, &ge, &tool_registry).await;

        drop(tui_tx);

        let events: Vec<TuiEvent> = {
            let mut v = Vec::new();
            while let Some(e) = tui_rx.recv().await {
                v.push(e);
            }
            v
        };

        let done_count = events
            .iter()
            .filter(|e| matches!(e, TuiEvent::Done))
            .count();
        assert_eq!(
            done_count, 1,
            "/help should emit exactly one Done (sent by the handler), got {done_count}"
        );
        assert!(
            events.iter().any(|e| matches!(e, TuiEvent::StreamChunk(_))),
            "/help should emit a StreamChunk with help text"
        );
    }

    #[tokio::test]
    async fn test_clear_handler_does_not_emit_done() {
        // /clear returns NeedsRender — it does NOT send Done. The structured
        // loop must send the Done itself after calling the handler.
        let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        let (mut app, ge, tool_registry, _dir) = make_deps();

        handle_slash_command("/clear", &mut app, &tui_tx, &ge, &tool_registry).await;

        drop(tui_tx);

        let events: Vec<TuiEvent> = {
            let mut v = Vec::new();
            while let Some(e) = tui_rx.recv().await {
                v.push(e);
            }
            v
        };

        let done_count = events
            .iter()
            .filter(|e| matches!(e, TuiEvent::Done))
            .count();
        assert_eq!(
            done_count, 0,
            "/clear should NOT emit any Done on its own (NeedsRender)"
        );
    }

    #[tokio::test]
    async fn test_structured_loop_skip_done_when_handler_emits_one() {
        // Simulate the exact fixed structured mode loop: for a Handled
        // command, the loop must NOT add a Done after the handler.
        let (tui_tx, mut tui_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
        let (mut app, ge, tool_registry, _dir) = make_deps();

        // Step 1: Submit event (as the structured loop does)
        tui_tx.send(TuiEvent::Submit("/help".into())).ok();

        // Step 2: Call handle_slash_command (sends its own Done internally)
        let outcome = handle_slash_command("/help", &mut app, &tui_tx, &ge, &tool_registry).await;

        // Step 3: The structured loop's fix — only send Done if NeedsRender
        if matches!(outcome, CommandOutcome::NeedsRender) {
            tui_tx.send(TuiEvent::Done).ok();
        }

        drop(tui_tx);

        let events: Vec<TuiEvent> = {
            let mut v = Vec::new();
            while let Some(e) = tui_rx.recv().await {
                v.push(e);
            }
            v
        };

        let done_count = events
            .iter()
            .filter(|e| matches!(e, TuiEvent::Done))
            .count();
        assert_eq!(
            done_count, 1,
            "Fixed loop: /help should produce exactly one Done event, got {done_count}"
        );
    }
}
