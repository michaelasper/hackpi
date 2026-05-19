pub mod command_gate;
pub mod config;
pub mod file_protection;
pub mod hot_reload;
pub mod path_guard;
pub mod pattern;
pub mod vcs_operation_gate;

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
    GitWriteOperation,
}

impl std::fmt::Display for GuardType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardType::PathAccess => write!(f, "PathAccess"),
            GuardType::CommandGate => write!(f, "CommandGate"),
            GuardType::FileProtection => write!(f, "FileProtection"),
            GuardType::GitWriteOperation => write!(f, "GitWriteOperation"),
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
    /// Optional operation filter (Read/Write) for file protection rules.
    /// When None, the rule applies to all operations.
    pub operation: Option<FileOp>,
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
    /// Examines the `command`, `path`, and (for git_write) `operation`
    /// parameters from the tool call, routing them through the appropriate
    /// guard components:
    /// - `command` → `command_gate`
    /// - `path` → `file_protection` + `path_guard`
    /// - `operation` (git_write) → `vcs_operation_gate`
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

            // Determine file operation type from tool name
            let op = match tool_name {
                "read" | "search_grep" | "searchgrep" => FileOp::Read,
                "write" | "edit" => FileOp::Write,
                _ => FileOp::Read, // unknown tools default to Read
            };

            // File protection check
            let fp_result = file_protection::check(path, &op, &self.config_rules, tool_name);
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

        // Check git_write operation gate (git_write tool)
        // Destructive operations like reset --hard, push --force, branch_delete,
        // merge, rebase, stash_pop, and checkout are guarded even when they
        // don't carry a guarded command/path parameter.
        if tool_name.eq_ignore_ascii_case("git_write") {
            if let Some(operation) = params.get("operation").and_then(|v| v.as_str()) {
                let result =
                    vcs_operation_gate::check(operation, params, &self.config_rules, tool_name);
                if !matches!(result, GuardResult::Allow) {
                    return result;
                }
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

    // ── Accessors for UI slash commands ───────────────────────────────────

    /// Return the number of currently loaded permission rules.
    pub fn rule_count(&self) -> usize {
        self.config_rules.len()
    }

    /// Return whether god mode is active.
    pub fn is_god_mode(&self) -> bool {
        self.god_mode
    }

    /// Return the number of entries in the session decision cache.
    pub fn session_cache_len(&self) -> usize {
        self.session_cache.len()
    }

    /// Persist an `AlwaysAllow` or `AlwaysDeny` decision to `.claude/settings.local.json`.
    ///
    /// `AlwaysAllow` entries are appended to `permissions.allow`.
    /// `AlwaysDeny` entries are appended to `permissions.deny`.
    ///
    /// Other decision variants return `Err` since they should not be persisted.
    pub fn persist_decision(
        &self,
        decision: &PermissionDecision,
        permission_string: &str,
    ) -> Result<(), String> {
        let target_array = match decision {
            PermissionDecision::AlwaysAllow => "allow",
            PermissionDecision::AlwaysDeny => "deny",
            _ => return Err("Only AlwaysAllow and AlwaysDeny decisions can be persisted".into()),
        };
        append_to_permissions(
            &self.settings_paths.claude_local,
            permission_string,
            target_array,
        )
    }

    /// Persist an `AlwaysAllow` or `AlwaysDeny` decision to `.hackpi/guardrails.json`.
    ///
    /// Same semantics as `persist_decision` but targets the project-wide config file.
    pub fn persist_to_hackpi_config(
        &self,
        decision: &PermissionDecision,
        permission_string: &str,
    ) -> Result<(), String> {
        let target_array = match decision {
            PermissionDecision::AlwaysAllow => "allow",
            PermissionDecision::AlwaysDeny => "deny",
            _ => return Err("Only AlwaysAllow and AlwaysDeny decisions can be persisted".into()),
        };
        append_to_permissions(&self.settings_paths.hackpi, permission_string, target_array)
    }

    /// Return a reference to the settings paths.
    pub fn settings_paths(&self) -> &SettingsPaths {
        &self.settings_paths
    }
}

/// Append a permission string to a JSON config file's permissions array.
///
/// Reads the file at `file_path` (or uses a default structure if the file
/// doesn't exist), navigates to `permissions.{target_array}`, checks for
/// duplicates (case-insensitive), appends the `permission_string` if not
/// already present, and writes the file back with pretty formatting.
///
/// # Errors
///
/// Returns `Err` if the file exists but cannot be read or parsed, or if
/// the file cannot be written.
pub fn append_to_permissions(
    file_path: &std::path::Path,
    permission_string: &str,
    target_array: &str,
) -> Result<(), String> {
    // Default structure for new file
    let default_value = serde_json::json!({
        "permissions": {
            "allow": [],
            "deny": []
        }
    });

    // Read existing or use default
    let mut config: serde_json::Value = if file_path.exists() {
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| format!("Failed to read {}: {e}", file_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {e}", file_path.display()))?
    } else {
        default_value.clone()
    };

    // Ensure permissions struct exists
    if !config.is_object() {
        config = default_value;
    }
    if !config
        .get("permissions")
        .and_then(|v| v.is_object().then_some(true))
        .unwrap_or(false)
    {
        config["permissions"] = serde_json::json!({"allow": [], "deny": []});
    }

    // Get target array
    let arr = config["permissions"][target_array]
        .as_array_mut()
        .ok_or_else(|| format!("permissions.{target_array} is not an array"))?;

    // Check for duplicate (case-insensitive for permission strings)
    if arr.iter().any(|v| {
        v.as_str()
            .map(|s| s.eq_ignore_ascii_case(permission_string))
            .unwrap_or(false)
    }) {
        return Ok(()); // Already exists, not an error
    }

    // Append
    arr.push(serde_json::Value::String(permission_string.to_string()));

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }

    // Write with pretty formatting — use temp file + atomic rename for crash safety
    let content =
        serde_json::to_string_pretty(&config).map_err(|e| format!("Failed to serialize: {e}"))?;

    // Write to a temporary file in the same directory, then atomically rename.
    // On POSIX, rename() is atomic when source and destination are on the same
    // filesystem, so concurrent readers always see a consistent file.
    let tmp_path = file_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write temporary file {}: {e}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, file_path).map_err(|e| {
        format!(
            "Failed to rename {} to {}: {e}",
            tmp_path.display(),
            file_path.display()
        )
    })?;

    Ok(())
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

    // ── PermissionRule Construction Tests ───────────────────────────────

    #[test]
    fn test_permission_rule_path_only() {
        let rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "./docs/**".into(),
            }),
            path_pattern: Some("./docs/**".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        };
        assert!(rule.path_pattern.is_some());
        assert!(rule.command_pattern.is_none());
    }

    #[test]
    fn test_permission_rule_command_only() {
        let rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "bash".into(),
                pattern: "curl *".into(),
            }),
            path_pattern: None,
            command_pattern: Some("curl *".into()),
            operation: None,
            action: RuleAction::Deny,
        };
        assert!(rule.path_pattern.is_none());
        assert!(rule.command_pattern.is_some());
    }

    #[test]
    fn test_permission_rule_both_patterns() {
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("./secrets/**".into()),
            command_pattern: Some("cat".into()),
            operation: None,
            action: RuleAction::Ask,
        };
        assert!(rule.path_pattern.is_some());
        assert!(rule.command_pattern.is_some());
    }

    #[test]
    fn test_permission_rule_no_patterns() {
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        };
        assert!(rule.path_pattern.is_none());
        assert!(rule.command_pattern.is_none());
    }

    #[test]
    fn test_permission_rule_serde_json_compatible() {
        // Verify PermissionRule fields are compatible with serde_json types
        // (even though RuleAction and ToolPattern don't derive Serialize/Deserialize)
        let _rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "./docs/**".into(),
            }),
            path_pattern: Some("./docs/**".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        };
        // Manually construct the JSON representation
        let json = serde_json::json!({
            "tool": "read",
            "path_pattern": "./docs/**",
            "action": "Allow"
        });
        assert_eq!(json["tool"].as_str(), Some("read"));
        assert_eq!(json["path_pattern"].as_str(), Some("./docs/**"));
        assert_eq!(json["action"].as_str(), Some("Allow"));
    }

    // ── check_tool Edge Cases ────────────────────────────────────────────

    #[test]
    fn test_check_tool_with_both_command_and_path_command_wins() {
        // When both command and path are present, command gate is checked first
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "command": "sudo rm -rf /",
            "path": "safe-file.txt"
        });
        let result = evaluator.check_tool("bash", &params);
        // Command gate should catch sudo before path guard is even reached
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("sudo") || msg.contains("dangerous"));
            }
            other => panic!("expected Deny for sudo command, got {other:?}"),
        }
    }

    /// An allow rule for a command should bypass dangerous patterns.
    #[test]
    fn test_check_tool_allow_rule_bypasses_dangerous_patterns() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        // Load a config that allows "sudo"
        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(&hackpi, r#"{"permissions": {"allow": ["Bash(sudo)"]}}"#).expect("write");

        evaluator.load_rules().expect("load rules");

        let params = json!({ "command": "sudo echo hello" });
        let result = evaluator.check_tool("bash", &params);
        assert_eq!(result, GuardResult::Allow);
    }

    // ── VCS Command Blocking Tests ────────────────────────────────────────

    #[test]
    fn test_bash_git_status_denied_by_default() {
        // Without any config, git commands in bash should be denied by built-in patterns
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({ "command": "git status" });
        let result = evaluator.check_tool("bash", &params);
        match result {
            GuardResult::Deny(msg) => {
                assert!(
                    msg.contains("git"),
                    "deny message should mention git: {msg}"
                );
            }
            other => panic!("expected Deny for 'git status', got {other:?}"),
        }
    }

    #[test]
    fn test_bash_gh_issue_list_denied_by_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({ "command": "gh issue list" });
        let result = evaluator.check_tool("bash", &params);
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("gh"), "deny message should mention gh: {msg}");
            }
            other => panic!("expected Deny for 'gh issue list', got {other:?}"),
        }
    }

    #[test]
    fn test_bash_ls_still_allowed_with_vcs_patterns() {
        // Non-VCS commands should still be allowed
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({ "command": "ls -la" });
        let result = evaluator.check_tool("bash", &params);
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_non_bash_tool_git_not_blocked() {
        // git commands in non-bash tools should NOT be blocked
        // (the command_gate checks all tools, but the built-in VCS deny
        // patterns should only apply when the tool is "bash")
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({ "command": "git status" });
        // Using a tool named "git_read" — should be allowed
        let result = evaluator.check_tool("git_read", &params);
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_allow_git_in_bash_true_bypasses_vcs_deny() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        // Create config with allow_git_in_bash: true
        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(
            &hackpi,
            r#"{"command_gate": {"allow_git_in_bash": true, "deny": ["git *", "gh *"]}}"#,
        )
        .expect("write");

        evaluator.load_rules().expect("load rules");

        let params = json!({ "command": "git status" });
        let result = evaluator.check_tool("bash", &params);
        assert_eq!(
            result,
            GuardResult::Allow,
            "git status should be allowed with allow_git_in_bash"
        );
    }

    #[test]
    fn test_allow_git_in_bash_true_allows_gh() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(&hackpi, r#"{"command_gate": {"allow_git_in_bash": true}}"#).expect("write");

        evaluator.load_rules().expect("load rules");

        let params = json!({ "command": "gh pr create" });
        let result = evaluator.check_tool("bash", &params);
        assert_eq!(
            result,
            GuardResult::Allow,
            "gh should be allowed with allow_git_in_bash"
        );
    }

    #[test]
    fn test_command_gate_allow_overrides_vcs_deny() {
        // A specific allow rule for "git status" should override the built-in VCS deny
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(
            &hackpi,
            r#"{"command_gate": {"allow": ["git status", "git log"], "deny": ["git *", "gh *"]}}"#,
        )
        .expect("write");

        evaluator.load_rules().expect("load rules");

        // "git status" should be allowed by the specific allow rule
        let params = json!({ "command": "git status" });
        let result = evaluator.check_tool("bash", &params);
        assert_eq!(
            result,
            GuardResult::Allow,
            "git status should be allowed by config allow rule"
        );
    }

    // ── GitWrite Operation Guardrail Tests ────────────────────────────────

    #[test]
    fn test_git_write_reset_hard_denied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "reset",
            "mode": "hard",
            "revision": "HEAD~1"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Deny(msg) => {
                assert!(
                    msg.contains("reset") || msg.contains("destructive"),
                    "deny msg should mention reset: {msg}"
                );
            }
            other => panic!("expected Deny for git_write reset --hard, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_force_push_denied() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "push",
            "force": true,
            "remote": "origin",
            "branch": "main"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("push") || msg_lower.contains("destructive"),
                    "deny msg should mention push: {msg}"
                );
            }
            other => panic!("expected Deny for git_write push --force, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_branch_delete_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "branch_delete",
            "branch": "old-feature"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("branch_delete"),
                    "ask msg should mention branch_delete: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for git_write branch_delete, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_merge_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "merge",
            "branch": "feature"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("merge"),
                    "ask msg should mention merge: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for git_write merge, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_rebase_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "rebase",
            "onto": "main"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("rebase"),
                    "ask msg should mention rebase: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for git_write rebase, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_stash_pop_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "stash_pop",
            "index": 0
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("stash_pop"),
                    "ask msg should mention stash_pop: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for git_write stash_pop, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_checkout_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let params = json!({
            "operation": "checkout",
            "branch": "main"
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("checkout"),
                    "ask msg should mention checkout: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for git_write checkout, got {other:?}"),
        }
    }

    #[test]
    fn test_git_write_add_and_commit_allowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        // add should be allowed (non-destructive)
        let params = json!({
            "operation": "add",
            "paths": ["src/main.rs"]
        });
        let result = evaluator.check_tool("git_write", &params);
        assert_eq!(result, GuardResult::Allow, "add should be allowed");

        // commit should be allowed (non-destructive)
        let params = json!({
            "operation": "commit",
            "message": "fix: something"
        });
        let result = evaluator.check_tool("git_write", &params);
        assert_eq!(result, GuardResult::Allow, "commit should be allowed");
    }

    #[test]
    fn test_git_write_allow_rule_overrides_destructive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        // Create config that allows reset
        // Note: permission string uses "git_write" (underscore) to match
        // the known tool name in the guard system
        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(
            &hackpi,
            r#"{"permissions": {"allow": ["git_write(reset)"]}}"#,
        )
        .expect("write");

        evaluator.load_rules().expect("load rules");

        let params = json!({
            "operation": "reset",
            "mode": "hard",
            "revision": "HEAD"
        });
        let result = evaluator.check_tool("git_write", &params);
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_git_write_deny_rule_overrides_default_allow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);

        // Create config that denies add
        let hackpi = dir.path().join(".hackpi/guardrails.json");
        std::fs::create_dir_all(hackpi.parent().unwrap()).expect("create dir");
        std::fs::write(&hackpi, r#"{"permissions": {"deny": ["git_write(add)"]}}"#).expect("write");

        evaluator.load_rules().expect("load rules");

        let params = json!({
            "operation": "add",
            "paths": ["."]
        });
        let result = evaluator.check_tool("git_write", &params);
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("add"), "deny msg should mention add: {msg}");
            }
            other => panic!("expected Deny for git_write add (config deny), got {other:?}"),
        }
    }

    // ── GuardType Display Tests ──────────────────────────────────────────

    #[test]
    fn test_guard_type_display() {
        assert_eq!(GuardType::PathAccess.to_string(), "PathAccess");
        assert_eq!(GuardType::CommandGate.to_string(), "CommandGate");
        assert_eq!(GuardType::FileProtection.to_string(), "FileProtection");
        assert_eq!(
            GuardType::GitWriteOperation.to_string(),
            "GitWriteOperation"
        );
    }

    // ── append_to_permissions Tests ──────────────────────────────────────

    // ── persist_decision Tests ───────────────────────────────────────────

    #[test]
    fn test_persist_decision_always_allow_creates_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let result =
            evaluator.persist_decision(&PermissionDecision::AlwaysAllow, "Read(./docs/**)");
        assert!(result.is_ok(), "should persist AlwaysAllow");

        // Verify the file was created at claude_local path
        let file_path = evaluator.settings_paths.claude_local.clone();
        assert!(file_path.exists(), "claude settings.local should exist");

        let content = std::fs::read_to_string(&file_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(allow.len(), 1);
        assert_eq!(allow[0].as_str(), Some("Read(./docs/**)"));
    }

    #[test]
    fn test_persist_decision_always_deny_appends_to_deny() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let result = evaluator.persist_decision(&PermissionDecision::AlwaysDeny, "Write(./.env)");
        assert!(result.is_ok(), "should persist AlwaysDeny");

        let file_path = evaluator.settings_paths.claude_local.clone();
        let content = std::fs::read_to_string(&file_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let deny = parsed["permissions"]["deny"]
            .as_array()
            .expect("deny array");
        assert_eq!(deny.len(), 1);
        assert_eq!(deny[0].as_str(), Some("Write(./.env)"));
    }

    #[test]
    fn test_persist_decision_non_persistable_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        // AllowOnce should not be persistable
        let result = evaluator.persist_decision(&PermissionDecision::AllowOnce, "Read(foo)");
        assert!(result.is_err(), "AllowOnce should not be persistable");
        assert!(
            result.unwrap_err().contains("Only AlwaysAllow"),
            "should explain valid types"
        );

        // AllowSession should not be persistable
        let result = evaluator.persist_decision(&PermissionDecision::AllowSession, "Read(foo)");
        assert!(result.is_err(), "AllowSession should not be persistable");
    }

    // ── persist_to_hackpi_config Tests ───────────────────────────────────

    #[test]
    fn test_persist_to_hackpi_config_always_deny() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let result =
            evaluator.persist_to_hackpi_config(&PermissionDecision::AlwaysDeny, "Bash(curl *)");
        assert!(result.is_ok(), "should persist to hackpi config");

        let hackpi_path = evaluator.settings_paths.hackpi.clone();
        assert!(hackpi_path.exists(), ".hackpi/guardrails.json should exist");

        let content = std::fs::read_to_string(&hackpi_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let deny = parsed["permissions"]["deny"]
            .as_array()
            .expect("deny array");
        assert_eq!(deny.len(), 1);
        assert_eq!(deny[0].as_str(), Some("Bash(curl *)"));
    }

    #[test]
    fn test_persist_to_hackpi_config_always_allow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);

        let result =
            evaluator.persist_to_hackpi_config(&PermissionDecision::AlwaysAllow, "Read(./src/**)");
        assert!(result.is_ok(), "should persist to hackpi config");

        let hackpi_path = evaluator.settings_paths.hackpi.clone();
        let content = std::fs::read_to_string(&hackpi_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(allow.len(), 1);
        assert_eq!(allow[0].as_str(), Some("Read(./src/**)"));
    }

    #[test]
    fn test_append_to_permissions_creates_new_file_with_allow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join(".claude/settings.local.json");

        let result = append_to_permissions(&file_path, "Read(./docs/**)", "allow");
        assert!(result.is_ok(), "should succeed creating new file");

        // Verify file was created with correct content
        let content = std::fs::read_to_string(&file_path).expect("file should exist");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("should be valid JSON");

        // Check structure
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("should have permissions.allow array");
        assert_eq!(allow.len(), 1, "should have one allow entry");
        assert_eq!(allow[0].as_str(), Some("Read(./docs/**)"));

        // Deny should be empty
        let deny = parsed["permissions"]["deny"]
            .as_array()
            .expect("should have permissions.deny array");
        assert!(deny.is_empty(), "deny should be empty");
    }

    #[test]
    fn test_append_to_permissions_duplicate_prevention() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("guardrails.json");

        // Create file with an existing entry
        let initial = r#"{"permissions":{"allow":["Read(./docs/**)"],"deny":[]}}"#;
        std::fs::write(&file_path, initial).expect("write initial");

        // Append the same entry again
        let result = append_to_permissions(&file_path, "Read(./docs/**)", "allow");
        assert!(result.is_ok(), "duplicate should not error");

        // Verify no duplicate was added
        let content = std::fs::read_to_string(&file_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(allow.len(), 1, "no duplicate should be added");
        assert_eq!(allow[0].as_str(), Some("Read(./docs/**)"));
    }

    #[test]
    fn test_append_to_permissions_duplicate_case_insensitive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("guardrails.json");

        // Create file with an existing entry in PascalCase
        let initial = r#"{"permissions":{"allow":["Read(./docs/**)"],"deny":[]}}"#;
        std::fs::write(&file_path, initial).expect("write initial");

        // Try to add with different case
        let result = append_to_permissions(&file_path, "read(./docs/**)", "allow");
        assert!(
            result.is_ok(),
            "case-insensitive duplicate should not error"
        );

        // Verify no duplicate was added
        let content = std::fs::read_to_string(&file_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(
            allow.len(),
            1,
            "case-insensitive duplicate should not be added"
        );
        // Original casing should be preserved
        assert_eq!(allow[0].as_str(), Some("Read(./docs/**)"));
    }

    #[test]
    fn test_append_to_permissions_invalid_json_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("bad.json");

        // Write invalid JSON
        std::fs::write(&file_path, "this is not json").expect("write invalid json");

        let result = append_to_permissions(&file_path, "Read(foo)", "allow");
        assert!(result.is_err(), "invalid JSON should return error");
        assert!(
            result.unwrap_err().contains("Failed to parse"),
            "error should mention parse failure"
        );
    }

    #[test]
    fn test_append_to_permissions_invalid_target_array() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("bad.json");

        // Create file with allow as a string instead of array
        let initial = r#"{"permissions":{"allow":"not_an_array","deny":[]}}"#;
        std::fs::write(&file_path, initial).expect("write initial");

        let result = append_to_permissions(&file_path, "Read(foo)", "allow");
        assert!(result.is_err(), "invalid array should return error");
        assert!(
            result.unwrap_err().contains("is not an array"),
            "error should mention array"
        );
    }

    #[test]
    fn test_append_to_permissions_atomic_write_no_temp_file_left_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("guardrails.json");

        // Create initial file
        let initial = r#"{"permissions":{"allow":[],"deny":[]}}"#;
        std::fs::write(&file_path, initial).expect("write initial");

        // Append a permission
        let result = append_to_permissions(&file_path, "Read(./docs/**)", "allow");
        assert!(result.is_ok(), "append should succeed");

        // Temp file should NOT exist after successful write
        let tmp_path = file_path.with_extension("json.tmp");
        assert!(
            !tmp_path.exists(),
            "temp file should be cleaned up after atomic write"
        );

        // The target file should have the correct content
        let content = std::fs::read_to_string(&file_path).expect("read file");
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse JSON");
        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("allow array");
        assert_eq!(allow.len(), 1, "should have one allow entry");
        assert_eq!(allow[0].as_str(), Some("Read(./docs/**)"));
    }

    #[test]
    fn test_append_to_permissions_write_failure_returns_error() {
        // Use a path where write is impossible (parent is a file, not a directory)
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("parent_file");
        std::fs::write(&file_path, "i am a file, not a dir").expect("write file");

        // Try to write to a "file" inside the parent_file "directory"
        let bad_path = file_path.join("nested.json");

        let result = append_to_permissions(&bad_path, "Read(foo)", "allow");
        assert!(result.is_err(), "write to bad path should return error");
    }

    #[test]
    fn test_append_to_permissions_appends_to_existing_deny() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("guardrails.json");

        // Create existing file with pre-populated allow
        let initial = r#"{"permissions":{"allow":["Read(./docs/**)"],"deny":[]}}"#;
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("create dir");
        }
        std::fs::write(&file_path, initial).expect("write initial");

        // Append a deny rule
        let result = append_to_permissions(&file_path, "Write(./.env)", "deny");
        assert!(result.is_ok(), "should append to existing file");

        // Verify file content
        let content = std::fs::read_to_string(&file_path).expect("file should exist");
        let parsed: serde_json::Value =
            serde_json::from_str(&content).expect("should be valid JSON");

        let allow = parsed["permissions"]["allow"]
            .as_array()
            .expect("should have allow array");
        assert_eq!(allow.len(), 1, "allow should preserve existing entry");
        assert_eq!(
            allow[0].as_str(),
            Some("Read(./docs/**)"),
            "allow entry unchanged"
        );

        let deny = parsed["permissions"]["deny"]
            .as_array()
            .expect("should have deny array");
        assert_eq!(deny.len(), 1, "deny should have one entry");
        assert_eq!(deny[0].as_str(), Some("Write(./.env)"), "deny entry added");
    }
}
