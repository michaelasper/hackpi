//! Reusable test helpers for guardrails integration and unit tests.
//!
//! Provides convenience functions for creating test configurations,
//! guard evaluators, and permission rules.

use hackpi_guardrails::{PermissionRule, RuleAction, SettingsPaths, ToolPattern};
use std::fs;
use std::path::Path;

/// Create a `PermissionRule` with the given parameters.
///
/// Pass `None` for `tool`, `path_pattern`, or `command_pattern` to omit them.
/// Tool name is lowercased automatically.
#[allow(dead_code)]
pub fn make_rule(
    tool: Option<&str>,
    path_pattern: Option<&str>,
    command_pattern: Option<&str>,
    action: RuleAction,
) -> PermissionRule {
    PermissionRule {
        tool_pattern: tool.map(|t| ToolPattern {
            name: t.to_lowercase(),
            pattern: path_pattern.or(command_pattern).unwrap_or("*").to_string(),
        }),
        path_pattern: path_pattern.map(|s| s.to_string()),
        command_pattern: command_pattern.map(|s| s.to_string()),
        operation: None,
        action,
    }
}

/// Create a temporary directory with a `.hackpi/guardrails.json` file.
///
/// Returns the `SettingsPaths` pointing into the temp dir and the
/// `tempfile::TempDir` guard (kept alive for the lifetime of the test).
pub fn create_test_config(
    hackpi_json: Option<&str>,
    claude_local_json: Option<&str>,
    claude_project_json: Option<&str>,
) -> (SettingsPaths, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let paths = SettingsPaths::new(dir.path());

    if let Some(content) = hackpi_json {
        write_config_file(&paths.hackpi, content);
    }
    if let Some(content) = claude_local_json {
        write_config_file(&paths.claude_local, content);
    }
    if let Some(content) = claude_project_json {
        write_config_file(&paths.claude_project, content);
    }

    (paths, dir)
}

/// Create a `SettingsPaths` and `GuardEvaluator` with pre-loaded rules.
///
/// Provide the JSON content for each config file (None = don't create).
/// The evaluator will have `load_rules()` called automatically.
pub fn create_guard_evaluator(
    god_mode: bool,
    hackpi_json: Option<&str>,
    claude_local_json: Option<&str>,
    claude_project_json: Option<&str>,
) -> (hackpi_guardrails::GuardEvaluator, tempfile::TempDir) {
    let (paths, dir) = create_test_config(hackpi_json, claude_local_json, claude_project_json);
    let mut evaluator = hackpi_guardrails::GuardEvaluator::new(god_mode, paths);
    evaluator.load_rules().expect("load_rules should succeed");
    (evaluator, dir)
}

/// Write a JSON string to a config file, creating parent directories as needed.
fn write_config_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create config dir");
    }
    fs::write(path, content).expect("failed to write config");
}
