use super::Assertions;
use crate::app::{App, UiStatus};
use ratatui::backend::TestBackend;
use std::path::Path;

/// Run assertions against the current app state and rendered buffer.
pub(super) fn run_assertions(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::Assertions;

    /// Helper to create a minimal App for assertion tests.
    fn make_app() -> App {
        App::new()
    }

    /// Helper to create a test terminal.
    fn make_terminal() -> ratatui::Terminal<TestBackend> {
        let backend = TestBackend::new(80, 24);
        ratatui::Terminal::new(backend).expect("test terminal")
    }

    #[test]
    fn test_assertion_conversation_len_ok() {
        let mut app = make_app();
        let terminal = make_terminal();

        // Add one conversation entry.
        app.handle_event(crate::events::TuiEvent::Submit("hello".into()));

        let assert = Assertions {
            conversation_len: Some(1),
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: None,
            files_exist: None,
        };

        assert!(run_assertions(&assert, &app, &terminal, 1, 1).is_ok());
    }

    #[test]
    fn test_assertion_conversation_len_mismatch() {
        let app = make_app();
        let terminal = make_terminal();

        let assert = Assertions {
            conversation_len: Some(99),
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: None,
            files_exist: None,
        };

        let result = run_assertions(&assert, &app, &terminal, 1, 1);
        assert!(result.is_err());
        let msg = format!("{}", result.err().unwrap());
        assert!(msg.contains("conversation_len"));
    }

    #[test]
    fn test_assertion_state_resting_is_idle() {
        let app = make_app(); // App::new() starts as Idle
        let terminal = make_terminal();

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: Some("Resting".into()),
            status_message: None,
            files_exist: None,
        };

        assert!(run_assertions(&assert, &app, &terminal, 1, 1).is_ok());
    }

    #[test]
    fn test_assertion_state_generating() {
        let mut app = make_app();
        let terminal = make_terminal();
        app.ui_status = UiStatus::Generating;

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: Some("Generating".into()),
            status_message: None,
            files_exist: None,
        };

        assert!(run_assertions(&assert, &app, &terminal, 1, 1).is_ok());
    }

    #[test]
    fn test_assertion_state_mismatch() {
        let app = make_app(); // Idle
        let terminal = make_terminal();

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: Some("Generating".into()),
            status_message: None,
            files_exist: None,
        };

        let result = run_assertions(&assert, &app, &terminal, 1, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_assertion_status_message_ok() {
        let mut app = make_app();
        let terminal = make_terminal();
        app.info_message = Some("All good".into());

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: Some("All good".into()),
            files_exist: None,
        };

        assert!(run_assertions(&assert, &app, &terminal, 1, 1).is_ok());
    }

    #[test]
    fn test_assertion_status_message_mismatch() {
        let mut app = make_app();
        let terminal = make_terminal();
        app.info_message = Some("Unexpected".into());

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: Some("Expected".into()),
            files_exist: None,
        };

        let result = run_assertions(&assert, &app, &terminal, 1, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_assertion_empty_status_message_with_none() {
        let app = make_app(); // info_message is None
        let terminal = make_terminal();

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: Some("".into()),
            files_exist: None,
        };

        assert!(run_assertions(&assert, &app, &terminal, 1, 1).is_ok());
    }

    #[test]
    fn test_assertion_files_exist_not_found() {
        let app = make_app();
        let terminal = make_terminal();

        let assert = Assertions {
            conversation_len: None,
            render_contains: None,
            not_render_contains: None,
            state: None,
            status_message: None,
            files_exist: Some(vec!["/tmp/nonexistent-assert-file.txt".into()]),
        };

        let result = run_assertions(&assert, &app, &terminal, 1, 1);
        assert!(result.is_err());
        let msg = format!("{}", result.err().unwrap());
        assert!(msg.contains("expected file"));
    }
}
