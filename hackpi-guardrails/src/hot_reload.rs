use std::sync::{Arc, RwLock};
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::config;
use crate::pattern;
use crate::PermissionRule;
use crate::SettingsPaths;

/// Watches config files and atomically swaps permission rules on change,
/// with validate-before-swap safety so invalid configs never corrupt the
/// active rule set.
pub struct HotReloader {
    /// Shared reference to the guard evaluator's rule list.
    rules: Arc<RwLock<Vec<PermissionRule>>>,
    /// Paths to the three config files to watch.
    settings_paths: SettingsPaths,
}

impl HotReloader {
    /// Create a new `HotReloader`.
    ///
    /// The `rules` arc is shared with the [`crate::GuardEvaluator`] so that
    /// reloaded rules become visible immediately.
    pub fn new(rules: Arc<RwLock<Vec<PermissionRule>>>, settings_paths: SettingsPaths) -> Self {
        Self {
            rules,
            settings_paths,
        }
    }

    /// Start the file watcher and begin hot-reloading rules.
    ///
    /// Spawns a background tokio task that watches the parent directories of
    /// all three config files. On each file-system event, the task debounces
    /// for 200 ms (batching rapid writes from editor saves), then calls
    /// [`try_reload`] to validate and atomically swap rules.
    ///
    /// Returns a `JoinHandle` that can be awaited or detached.
    pub fn start(self) -> Result<JoinHandle<()>, String> {
        let (tx, rx) = std::sync::mpsc::channel::<Result<notify::Event, notify::Error>>();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .map_err(|e| format!("Failed to create watcher: {e}"))?;

        // Watch parent directories of each config file.
        // Create parent directories first so notify has a valid path to watch,
        // even if the config file doesn't exist yet (it may be created later).
        let watch_dirs = [
            &self.settings_paths.hackpi,
            &self.settings_paths.claude_local,
            &self.settings_paths.claude_project,
        ];
        for path in &watch_dirs {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
                watcher
                    .watch(parent, RecursiveMode::NonRecursive)
                    .map_err(|e| format!("Failed to watch {parent:?}: {e}"))?;
            }
        }

        let rules = self.rules;
        let settings_paths = self.settings_paths;

        let handle = tokio::spawn(async move {
            // Bridge from std::sync::mpsc (notify uses OS threads) to
            // tokio::sync::mpsc so we can receive events asynchronously.
            let (async_tx, mut async_rx) = mpsc::unbounded_channel::<notify::Event>();

            // Keep the watcher alive in a dedicated OS thread.
            std::thread::spawn(move || {
                let _watcher = watcher;
                while let Ok(Ok(event)) = rx.recv() {
                    if async_tx.send(event).is_err() {
                        break;
                    }
                }
            });

            // Event loop: debounce and reload
            while async_rx.recv().await.is_some() {
                // Debounce: wait 200 ms to batch rapid writes
                // (e.g. editor saves that generate multiple events).
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Drain any events that arrived during the sleep
                while async_rx.try_recv().is_ok() {}

                // Validate-before-swap reload
                if let Err(e) = try_reload(&rules, &settings_paths) {
                    tracing::error!("Hot reload failed: {e}");
                }
            }
        });

        Ok(handle)
    }
}

/// Load, validate, and atomically swap permission rules.
///
/// 1. Loads rules from all config files via [`config::load_all`].
/// 2. Validates the new rules via [`validate`].
/// 3. On success: atomically swaps the rules in the shared `Arc<RwLock<...>>`.
/// 4. On failure: logs the error and **does not** modify the active rules.
pub fn try_reload(
    rules: &Arc<RwLock<Vec<PermissionRule>>>,
    settings_paths: &SettingsPaths,
) -> Result<(), String> {
    let new_rules = config::load_all(settings_paths)?;
    validate(&new_rules)?;

    // Atomic swap — old rules remain valid and readable during the write
    let mut guard = rules.write().map_err(|e| format!("Lock poisoned: {e}"))?;
    *guard = new_rules;

    tracing::info!("Hot reload: rules updated successfully");
    Ok(())
}

/// Validate a set of permission rules without applying them.
///
/// Checks that:
/// - All glob patterns compile with `globset::Glob::new()`.
/// - Tool names (when present) are known tools.
/// - Command patterns are non-empty.
///
/// Structural JSON errors are already caught by serde during parsing
/// and are NOT checked here.
pub fn validate(rules: &[PermissionRule]) -> Result<(), String> {
    for (i, rule) in rules.iter().enumerate() {
        // Validate glob patterns compile
        if let Some(path_pattern) = &rule.path_pattern {
            pattern::compile_glob(path_pattern)
                .map_err(|e| format!("Rule {i}: invalid glob pattern '{path_pattern}': {e}"))?;
        }

        // Validate tool names are known (case-insensitive)
        if let Some(tp) = &rule.tool_pattern {
            if !pattern::is_known_tool(&tp.name) {
                return Err(format!("Rule {i}: unknown tool name '{}'", tp.name));
            }
        }

        // Validate command patterns are non-empty
        if let Some(cmd_pattern) = &rule.command_pattern {
            if cmd_pattern.is_empty() {
                return Err(format!("Rule {i}: command pattern is empty"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionRule, RuleAction, SettingsPaths};
    use std::fs;
    use std::sync::{Arc, RwLock};

    // ── Helper ────────────────────────────────────────────────────────────

    fn write_config(path: &std::path::Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create config dir");
        }
        fs::write(path, content).expect("failed to write config");
    }

    fn make_paths(root: &std::path::Path) -> SettingsPaths {
        SettingsPaths {
            hackpi: root.join(".hackpi/guardrails.json"),
            claude_local: root.join(".claude/settings.local.json"),
            claude_project: root.join(".claude/settings.json"),
        }
    }

    // ── validate() tests ──────────────────────────────────────────────────

    #[test]
    fn test_validate_valid_rules_ok() {
        let rules = vec![
            PermissionRule {
                tool_pattern: Some(crate::ToolPattern {
                    name: "read".into(),
                    pattern: "./docs/**".into(),
                }),
                path_pattern: Some("./docs/**".into()),
                command_pattern: None,
                action: RuleAction::Allow,
            },
            PermissionRule {
                tool_pattern: None,
                path_pattern: None,
                command_pattern: Some("npm install".into()),
                action: RuleAction::Deny,
            },
        ];
        assert!(validate(&rules).is_ok());
    }

    #[test]
    fn test_validate_empty_rules_ok() {
        let rules: Vec<PermissionRule> = Vec::new();
        assert!(validate(&rules).is_ok());
    }

    #[test]
    fn test_validate_invalid_glob_err() {
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("[invalid-glob".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("invalid glob"),
            "error should mention invalid glob"
        );
    }

    #[test]
    fn test_validate_unknown_tool_err() {
        let rules = vec![PermissionRule {
            tool_pattern: Some(crate::ToolPattern {
                name: "unknown_tool".into(),
                pattern: "*".into(),
            }),
            path_pattern: Some("./foo".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("unknown tool"),
            "error should mention unknown tool"
        );
    }

    #[test]
    fn test_validate_empty_command_pattern_err() {
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("".into()),
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("empty"),
            "error should mention empty"
        );
    }

    #[test]
    fn test_validate_none_path_and_command_ok() {
        // A rule with no path_pattern and no command_pattern is unusual
        // but not invalid per the validation rules
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        }];
        assert!(validate(&rules).is_ok());
    }

    #[test]
    fn test_validate_valid_glob_with_special_chars_ok() {
        // Patterns with [a-z] or ? should compile
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/[a-z]*/?.txt".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        assert!(validate(&rules).is_ok());
    }

    // ── try_reload() tests ────────────────────────────────────────────────

    #[test]
    fn test_try_reload_no_config_files_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // No config files exist → should succeed with empty rules
        let result = try_reload(&rules, &paths);
        assert!(result.is_ok());

        let guard = rules.read().expect("lock");
        assert!(guard.is_empty(), "no config files → empty rules");
    }

    #[test]
    fn test_try_reload_with_valid_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // Create a hackpi config file
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"], "deny": ["Write(./.env)"]}}"#,
        );

        let result = try_reload(&rules, &paths);
        assert!(result.is_ok());

        let guard = rules.read().expect("lock");
        assert_eq!(guard.len(), 2, "should have 2 rules from hackpi config");
        assert!(guard.iter().any(|r| r.action == RuleAction::Deny));
        assert!(guard.iter().any(|r| r.action == RuleAction::Allow));
    }

    #[test]
    fn test_try_reload_preserves_old_rules_on_invalid_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // First, load valid rules
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );
        try_reload(&rules, &paths).expect("first reload should succeed");
        assert_eq!(
            rules.read().expect("lock").len(),
            1,
            "should have 1 valid rule"
        );

        // Now write an invalid config (bad glob pattern)
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read([invalid-glob)"]}}"#,
        );
        let result = try_reload(&rules, &paths);
        assert!(result.is_err(), "invalid config should fail validation");

        // Old rules must be preserved
        let guard = rules.read().expect("lock");
        assert_eq!(guard.len(), 1, "old rules should be preserved");
    }

    #[test]
    fn test_try_reload_preserves_old_rules_on_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // First, load valid rules
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );
        try_reload(&rules, &paths).expect("first reload should succeed");

        // Now write invalid JSON
        write_config(&paths.hackpi, "not valid json at all");
        let result = try_reload(&rules, &paths);
        assert!(result.is_err(), "invalid JSON should fail");

        // Old rules must be preserved
        let guard = rules.read().expect("lock");
        assert_eq!(guard.len(), 1, "old rules should be preserved");
    }

    #[test]
    fn test_validate_rejects_empty_command_pattern() {
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("".into()),
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("empty"),
            "should reject empty command pattern"
        );
    }

    #[test]
    fn test_validate_rejects_unknown_tool_name() {
        let rules = vec![PermissionRule {
            tool_pattern: Some(crate::ToolPattern {
                name: "nonexistent_tool".into(),
                pattern: "*".into(),
            }),
            path_pattern: Some("./foo".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("unknown tool"),
            "should reject unknown tool"
        );
    }

    #[test]
    fn test_validate_rejects_invalid_glob_with_brackets() {
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("[invalid-glob".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let result = validate(&rules);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("invalid glob"),
            "should reject invalid glob pattern"
        );
    }

    #[test]
    fn test_try_reload_preserves_old_rules_when_new_config_is_missing_required_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // First, load valid rules
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );
        try_reload(&rules, &paths).expect("first reload should succeed");
        assert_eq!(
            rules.read().expect("lock").len(),
            1,
            "should have 1 valid rule initially"
        );

        // Now write a config with an invalid glob
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read([bad-glob)"]}}"#,
        );
        let result = try_reload(&rules, &paths);
        assert!(result.is_err(), "invalid glob config should fail");

        // Old rules must be preserved
        let guard = rules.read().expect("lock");
        assert_eq!(
            guard.len(),
            1,
            "old rules should be preserved after glob validation failure"
        );
        assert_eq!(guard[0].action, RuleAction::Allow);
    }

    #[test]
    fn test_try_reload_atomic_swap_visible_immediately() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // Load first set of rules
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );
        try_reload(&rules, &paths).expect("first reload");
        assert_eq!(
            rules.read().expect("lock").len(),
            1,
            "1 rule after first reload"
        );

        // Load different rules
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"deny": ["Write(./.env)"]}}"#,
        );
        try_reload(&rules, &paths).expect("second reload");

        // Must have exactly 1 rule, and it must be the new one
        let guard = rules.read().expect("lock");
        assert_eq!(guard.len(), 1, "1 rule after second reload");
        assert_eq!(
            guard[0].action,
            RuleAction::Deny,
            "new rules should replace old rules"
        );
    }

    #[test]
    fn test_try_reload_merge_three_configs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // Write to all three config files
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"deny": ["Write(./.env)"]}}"#,
        );
        write_config(
            &paths.claude_local,
            r#"{"permissions": {"allow": ["Read(./tmp/**)"]}}"#,
        );
        write_config(
            &paths.claude_project,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );

        let result = try_reload(&rules, &paths);
        assert!(result.is_ok());

        let guard = rules.read().expect("lock");
        // All 3 config files merged: 1 deny + 1 allow + 1 allow = 3
        assert_eq!(guard.len(), 3, "should merge rules from all 3 configs");
    }

    // ── HotReloader integration tests ─────────────────────────────────────

    #[tokio::test]
    #[cfg_attr(
        target_os = "linux",
        ignore = "notify may not work reliably in CI on Linux"
    )]
    async fn test_watcher_detects_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // Create initial config and pre-load it
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./initial/**)"]}}"#,
        );
        try_reload(&rules, &paths).expect("initial load should succeed");

        // Start the reloader
        let reloader = HotReloader::new(Arc::clone(&rules), paths);
        let handle = reloader.start().expect("failed to start reloader");

        // Give the watcher time to start and do initial scan
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Modify the config file
        write_config(
            &dir.path().join(".hackpi/guardrails.json"),
            r#"{"permissions": {"deny": ["Write(./updated/**)"]}}"#,
        );

        // Wait for the watcher to detect the change and reload
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify rules were updated
        {
            let guard = rules.read().expect("lock");
            assert_eq!(guard.len(), 1, "should have 1 rule after watcher reload");
            assert_eq!(
                guard[0].action,
                RuleAction::Deny,
                "rules should reflect the updated config"
            );
        }

        // Clean up
        handle.abort();
    }

    #[tokio::test]
    #[cfg_attr(
        target_os = "linux",
        ignore = "notify may not work reliably in CI on Linux"
    )]
    async fn test_watcher_does_not_update_on_invalid_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = Arc::new(RwLock::new(Vec::new()));

        // Create valid initial config and pre-load it so the rules are
        // known before the watcher starts.
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./safe/**)"]}}"#,
        );

        // Pre-load rules so they're populated before the watcher
        try_reload(&rules, &paths).expect("initial load should succeed");

        // Start the reloader
        let reloader = HotReloader::new(Arc::clone(&rules), paths);
        let handle = reloader.start().expect("failed to start reloader");

        // Wait for watcher to start
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Write invalid config
        write_config(
            &dir.path().join(".hackpi/guardrails.json"),
            "not valid json",
        );

        // Wait for the watcher to detect the change and fail to reload
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Old rules must be preserved
        {
            let guard = rules.read().expect("lock");
            assert!(!guard.is_empty(), "old rules should be preserved");
            assert_eq!(
                guard[0].action,
                RuleAction::Allow,
                "original allow rule should remain"
            );
        }

        handle.abort();
    }
}
