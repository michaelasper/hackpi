pub mod assertions;
pub mod loader;
pub mod runner;

use serde::Deserialize;

// ── Re-exports ─────────────────────────────────────────────────────────────
pub use loader::parse_script_args;
pub use runner::run_scenario;

// ── Core types ─────────────────────────────────────────────────────────────

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

/// Parse a key identifier string into a `crossterm::event::KeyEvent`.
///
/// Supported values:
/// - `Up`, `Down`, `Enter`, `Esc`, `Tab`, `Backspace`, `Delete`
/// - `PageUp`, `PageDown`, `Home`, `End`
/// - `CtrlC`, `CtrlD`, `CtrlL`
/// - `Char(c)` where `c` is a single character (e.g. `Char(n)`)
pub(super) fn parse_key(s: &str) -> Option<crossterm::event::KeyEvent> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
pub(super) fn action_label(action: &Action) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::path::Path;

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

    // ── Scenario runner integration tests ───────────────────────────────

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
