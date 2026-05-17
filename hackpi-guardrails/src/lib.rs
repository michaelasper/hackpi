pub mod command_gate;
pub mod config;
pub mod file_protection;
pub mod hot_reload;
pub mod path_guard;
pub mod pattern;

use std::collections::HashMap;
use std::path::PathBuf;

// ── Core Types ──────────────────────────────────────────────────────────────

/// The result of a guard check.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardResult {
    /// The tool call is allowed to proceed.
    Allow,
    /// The tool call is denied with a reason message.
    Deny(String),
    /// The tool call should prompt the user with the given reason.
    Ask(GuardReason),
}

/// Describes why a guard prompted or denied a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardReason {
    /// Which guard component triggered this reason.
    pub guard: GuardType,
    /// The name of the tool being checked.
    pub tool: String,
    /// Human-readable details about why it was flagged.
    pub details: String,
}

/// Identifies which guard component produced a result.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardType {
    PathAccess,
    CommandGate,
    FileProtection,
}

impl std::fmt::Display for GuardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardType::PathAccess => write!(f, "PathAccess"),
            GuardType::CommandGate => write!(f, "CommandGate"),
            GuardType::FileProtection => write!(f, "FileProtection"),
        }
    }
}

/// The user's decision after being prompted.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    AllowOnce,
    AllowSession,
    Deny,
    AlwaysAllow,
    AlwaysDeny,
}

/// The action a rule takes when matched.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleAction {
    Allow,
    Deny,
    Ask,
}

/// A parsed tool+pattern pair, e.g. `Read("./docs/**")`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolPattern {
    pub name: String,
    pub pattern: String,
}

/// A single permission rule loaded from config.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRule {
    /// Optional tool filter — None means applies to all tools.
    pub tool_pattern: Option<ToolPattern>,
    /// Optional path glob pattern — None means command-only rules.
    pub path_pattern: Option<String>,
    /// Optional command substring pattern — None means path-only rules.
    pub command_pattern: Option<String>,
    /// What to do when this rule matches.
    pub action: RuleAction,
}

/// File operation type for protection rules.
#[derive(Debug, Clone, PartialEq)]
pub enum FileOp {
    Read,
    Write,
}

/// Paths to the three config files used by the guard system.
#[derive(Debug, Clone)]
pub struct SettingsPaths {
    pub hackpi: PathBuf,         // .hackpi/guardrails.json
    pub claude_local: PathBuf,   // .claude/settings.local.json
    pub claude_project: PathBuf, // .claude/settings.json
}

impl SettingsPaths {
    /// Create a new `SettingsPaths` from a workspace root directory.
    pub fn new(workspace_root: &std::path::Path) -> Self {
        Self {
            hackpi: workspace_root.join(".hackpi/guardrails.json"),
            claude_local: workspace_root.join(".claude/settings.local.json"),
            claude_project: workspace_root.join(".claude/settings.json"),
        }
    }
}

/// The main guard evaluation engine.
///
/// Checks tool calls against loaded permission rules and provides
/// session-level caching for user decisions.
#[derive(Debug)]
pub struct GuardEvaluator {
    /// If true, all guard checks are bypassed.
    god_mode: bool,
    /// Loaded permission rules.
    config_rules: Vec<PermissionRule>,
    /// Session-level cache of user decisions.
    session_cache: HashMap<String, PermissionDecision>,
    /// Paths to config files.
    settings_paths: SettingsPaths,
    /// The root directory of the workspace (used by path_guard).
    workspace_root: PathBuf,
}

impl GuardEvaluator {
    /// Create a new `GuardEvaluator`.
    ///
    /// When `god_mode` is true, all checks return `Allow` unconditionally.
    pub fn new(god_mode: bool, settings_paths: SettingsPaths) -> Self {
        let workspace_root = settings_paths
            .hackpi
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            god_mode,
            config_rules: Vec::new(),
            session_cache: HashMap::new(),
            settings_paths,
            workspace_root,
        }
    }

    /// Check whether a tool call is allowed.
    ///
    /// Examines the `command` and `path` parameters from the tool call,
    /// routing them through the appropriate guard components:
    /// - `command` → `command_gate`
    /// - `path` → `file_protection` + `path_guard`
    ///
    /// Returns `Allow` if all guards pass, `Deny` with a reason if any
    /// guard blocks, or `Ask` with a reason if user input is needed.
    pub fn check_tool(&self, tool_name: &str, params: &serde_json::Value) -> GuardResult {
        if self.god_mode {
            return GuardResult::Allow;
        }

        // Check command gate (bash tool)
        if let Some(command) = params.get("command").and_then(|v| v.as_str()) {
            let result = command_gate::check(command, &self.config_rules, tool_name);
            if !matches!(result, GuardResult::Allow) {
                return result;
            }
        }

        // Check file protection + path guard (tools with a path parameter)
        if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
            let path = std::path::Path::new(path);

            // File protection check
            let fp_result = file_protection::check(path, &self.config_rules);
            if !matches!(fp_result, GuardResult::Allow) {
                return fp_result;
            }

            // Path guard check
            let pg_result = path_guard::check(
                &path.to_string_lossy(),
                &self.workspace_root,
                &self.config_rules,
                tool_name,
            );
            if !matches!(pg_result, GuardResult::Allow) {
                return pg_result;
            }
        }

        GuardResult::Allow
    }

    /// Record a user decision in the session cache.
    ///
    /// `AllowSession` and `Deny` (without persistence) decisions are stored
    /// for the duration of the session. `AlwaysAllow` and `AlwaysDeny` are
    /// intentionally *not* cached here — they should be persisted to config.
    pub fn record_decision(&mut self, key: String, decision: PermissionDecision) {
        match decision {
            PermissionDecision::AllowSession => {
                self.session_cache.insert(key, decision);
            }
            PermissionDecision::Deny => {
                self.session_cache.insert(key, decision);
            }
            // AllowOnce: don't cache
            // AlwaysAllow / AlwaysDeny: persisted to config, not session cache
            _ => {}
        }
    }

    /// Look up a cached session decision for the given key.
    pub fn session_decision(&self, key: &str) -> Option<&PermissionDecision> {
        self.session_cache.get(key)
    }

    /// Clear all session-level decisions.
    pub fn clear_session(&mut self) {
        self.session_cache.clear();
    }

    /// Load rules from all config files.
    ///
    /// Calls `config::load_all()` to parse and merge rules from
    /// `.hackpi/guardrails.json`, `.claude/settings.json`, and
    /// `.claude/settings.local.json`.
    pub fn load_rules(&mut self) -> Result<(), String> {
        let rules = config::load_all(&self.settings_paths)?;
        self.config_rules = rules;
        Ok(())
    }

    /// Attempt to reload rules from config files, keeping old rules on failure.
    ///
    /// This is the validate-before-swap entry point for hot reload.
    /// Returns `Ok(())` on success, `Err(reason)` if the new config is invalid
    /// (the old rules are preserved).
    pub fn try_reload(&mut self) -> Result<(), String> {
        let new_rules = config::load_all(&self.settings_paths)?;
        hot_reload::validate(&new_rules)?;
        self.config_rules = new_rules;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── God Mode Tests ───────────────────────────────────────────────────

    #[test]
    fn test_god_mode_bypasses_all_checks() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let evaluator = GuardEvaluator::new(true, paths);

        // Even a suspicious command should be allowed in god mode
        let params = json!({ "command": "rm -rf /" });
        assert_eq!(evaluator.check_tool("bash", &params), GuardResult::Allow);

        // Even a protected path should be allowed in god mode
        let params = json!({ "path": ".env" });
        assert_eq!(evaluator.check_tool("read", &params), GuardResult::Allow);
    }

    #[test]
    fn test_god_mode_stored_and_accessible() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let evaluator = GuardEvaluator::new(true, paths);
        assert!(evaluator.god_mode);

        let paths2 = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let evaluator2 = GuardEvaluator::new(false, paths2);
        assert!(!evaluator2.god_mode);
    }

    // ── Session Cache Tests ──────────────────────────────────────────────

    #[test]
    fn test_session_cache_allow_session() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let mut evaluator = GuardEvaluator::new(false, paths);

        assert!(evaluator.session_decision("test-key").is_none());

        evaluator.record_decision("test-key".into(), PermissionDecision::AllowSession);
        assert_eq!(
            evaluator.session_decision("test-key"),
            Some(&PermissionDecision::AllowSession)
        );
    }

    #[test]
    fn test_session_cache_deny() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let mut evaluator = GuardEvaluator::new(false, paths);

        evaluator.record_decision("deny-key".into(), PermissionDecision::Deny);
        assert_eq!(
            evaluator.session_decision("deny-key"),
            Some(&PermissionDecision::Deny)
        );
    }

    #[test]
    fn test_session_cache_allow_once_not_cached() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let mut evaluator = GuardEvaluator::new(false, paths);

        evaluator.record_decision("once-key".into(), PermissionDecision::AllowOnce);
        assert!(evaluator.session_decision("once-key").is_none());
    }

    #[test]
    fn test_clear_session() {
        let paths = SettingsPaths::new(std::path::Path::new("/tmp/test"));
        let mut evaluator = GuardEvaluator::new(false, paths);

        evaluator.record_decision("key1".into(), PermissionDecision::AllowSession);
        evaluator.record_decision("key2".into(), PermissionDecision::Deny);
        evaluator.clear_session();

        assert!(evaluator.session_decision("key1").is_none());
        assert!(evaluator.session_decision("key2").is_none());
    }

    // ── Empty Config Reload Test ─────────────────────────────────────────

    #[test]
    fn test_load_rules_empty_config() {
        // Use a temp directory with no config files
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        // Should succeed with empty rules (no config files to parse yet)
        let result = evaluator.load_rules();
        assert!(result.is_ok());
        assert!(evaluator.config_rules.is_empty());
    }

    #[test]
    fn test_try_reload_with_empty_config() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        let result = evaluator.try_reload();
        assert!(result.is_ok());
    }

    // ── Config Rules Field Tests ─────────────────────────────────────────

    #[test]
    fn test_check_tool_non_god_mode_passes_with_no_rules() {
        // Use a temp dir so that path_guard can canonicalize the workspace root
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        // With no rules loaded, everything should be allowed
        let params = json!({ "command": "echo hello" });
        assert_eq!(evaluator.check_tool("bash", &params), GuardResult::Allow);

        // Create a file inside the workspace so canonicalize works
        let inside_path = dir.path().join("test.txt");
        std::fs::write(&inside_path, "content").expect("write test file");
        let params = json!({ "path": inside_path.to_str().unwrap() });
        assert_eq!(evaluator.check_tool("read", &params), GuardResult::Allow);
    }

    // ── SettingsPaths Tests ──────────────────────────────────────────────

    #[test]
    fn test_settings_paths_new() {
        let root = std::path::Path::new("/workspace/my-project");
        let paths = SettingsPaths::new(root);

        assert_eq!(paths.hackpi, root.join(".hackpi/guardrails.json"));
        assert_eq!(paths.claude_local, root.join(".claude/settings.local.json"));
        assert_eq!(paths.claude_project, root.join(".claude/settings.json"));
    }

    // ── GuardType Display Tests ──────────────────────────────────────────

    #[test]
    fn test_guard_type_display() {
        assert_eq!(GuardType::PathAccess.to_string(), "PathAccess");
        assert_eq!(GuardType::CommandGate.to_string(), "CommandGate");
        assert_eq!(GuardType::FileProtection.to_string(), "FileProtection");
    }
}
