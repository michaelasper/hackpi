use crate::{PermissionRule, RuleAction, SettingsPaths};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Load and merge permission rules from all configured sources.
///
/// Reads `.hackpi/guardrails.json`, `.claude/settings.local.json`, and
/// `.claude/settings.json`, parses them, and merges by priority.
///
/// Priority (highest first):
/// 1. `.hackpi/guardrails.json` — project-specific, committed
/// 2. `.claude/settings.local.json` — personal overrides, gitignored
/// 3. `.claude/settings.json` — team-wide defaults, checked in
///
/// Non-existent files are silently skipped.
/// Returns an empty Vec if no config files exist.
pub fn load_all(paths: &SettingsPaths) -> Result<Vec<PermissionRule>, String> {
    let mut all_rules = Vec::new();

    // 1. .hackpi/guardrails.json (highest priority)
    if paths.hackpi.exists() {
        let rules = load_hackpi_config(&paths.hackpi)?;
        all_rules.extend(rules);
    }

    // 2. .claude/settings.local.json
    if paths.claude_local.exists() {
        let rules = load_claude_settings(&paths.claude_local)?;
        all_rules.extend(rules);
    }

    // 3. .claude/settings.json (lowest priority)
    if paths.claude_project.exists() {
        let rules = load_claude_settings(&paths.claude_project)?;
        all_rules.extend(rules);
    }

    Ok(all_rules)
}

/// Parse `.hackpi/guardrails.json` format.
///
/// Supports four sections:
/// - `permissions` — allow/deny arrays of `ToolName(pattern)` strings
/// - `path_access` — path-based allow/deny with optional catch-all ask
/// - `command_gate` — command pattern map with ask/deny actions
/// - `file_protection` — file pattern map with per-operation actions
pub fn load_hackpi_config(path: &Path) -> Result<Vec<PermissionRule>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let config: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

    let mut rules: Vec<PermissionRule> = Vec::new();

    // Parse permissions block
    if let Some(perms) = config.get("permissions") {
        let allow = perms.get("allow").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
        let deny = perms.get("deny").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

        if let Ok(perm_rules) = parse_permissions_block(allow.as_deref(), deny.as_deref()) {
            rules.extend(perm_rules);
        }
    }

    // Parse path_access block
    if let Some(pa) = config.get("path_access") {
        if let Ok(pa_rules) = parse_path_access_block(pa) {
            rules.extend(pa_rules);
        }
    }

    // Parse command_gate block
    if let Some(cg) = config.get("command_gate") {
        if let Ok(cg_rules) = parse_command_gate_block(cg) {
            rules.extend(cg_rules);
        }
    }

    // Parse file_protection block
    if let Some(fp) = config.get("file_protection") {
        if let Ok(fp_rules) = parse_file_protection_block(fp) {
            rules.extend(fp_rules);
        }
    }

    Ok(rules)
}

/// Parse `.claude/settings.json` or `.claude/settings.local.json`.
///
/// Extracts the `permissions` block with allow/deny arrays of
/// `ToolName(pattern)` strings.
pub fn load_claude_settings(path: &Path) -> Result<Vec<PermissionRule>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let config: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

    let mut rules = Vec::new();

    if let Some(perms) = config.get("permissions") {
        let allow = perms.get("allow").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
        let deny = perms.get("deny").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

        if let Ok(perm_rules) = parse_permissions_block(allow.as_deref(), deny.as_deref()) {
            rules.extend(perm_rules);
        }
    }

    Ok(rules)
}

/// Parse Claude Code-style permission strings.
///
/// Each string has the format `ToolName(pattern)` (e.g. `Read(./.env)`).
/// Malformed entries are silently skipped (logged as warnings).
///
/// Returns rules ordered with deny first, then allow (so that within a
/// single source, deny rules take precedence during first-match evaluation).
pub fn parse_permissions_block(
    allow: Option<&[String]>,
    deny: Option<&[String]>,
) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    // Deny rules first (deny beats allow within a source)
    if let Some(deny_strs) = deny {
        for s in deny_strs {
            if let Some(rule) = permission_string_to_rule(s, RuleAction::Deny) {
                rules.push(rule);
            }
        }
    }

    // Allow rules second
    if let Some(allow_strs) = allow {
        for s in allow_strs {
            if let Some(rule) = permission_string_to_rule(s, RuleAction::Allow) {
                rules.push(rule);
            }
        }
    }

    Ok(rules)
}

/// Convert a single permission string and action into a PermissionRule.
///
/// Returns None if the string is malformed (invalid format, unknown tool, etc.).
fn permission_string_to_rule(s: &str, action: RuleAction) -> Option<PermissionRule> {
    let parsed = crate::pattern::parse_permission_string(s)?;

    let (tool_pattern_str, pattern) = parsed;

    let tool_pattern = tool_pattern_str.map(|tp| crate::ToolPattern {
        name: tp.name,
        pattern: tp.pattern,
    });

    // Determine if this is a path or command pattern based on the tool
    let is_path_tool = tool_pattern
        .as_ref()
        .map(|tp| {
            matches!(
                tp.name.as_str(),
                "read" | "write" | "edit" | "search_grep" | "searchgrep"
            )
        })
        .unwrap_or(true);

    Some(PermissionRule {
        tool_pattern,
        path_pattern: if is_path_tool {
            Some(pattern.clone())
        } else {
            None
        },
        command_pattern: if !is_path_tool { Some(pattern) } else { None },
        action,
    })
}

/// Parse the `path_access` config block.
///
/// Structure:
/// ```json
/// {
///   "allow": ["~/.config/**", "/tmp/**"],
///   "deny": ["/etc/**", "/usr/**"],
///   "ask": true
/// }
/// ```
///
/// Path glob patterns without a tool prefix apply to all tools.
/// If `ask: true`, a catch-all Ask rule is appended so that paths
/// not matching any allow/deny rule will prompt the user.
pub fn parse_path_access_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    // Deny rules first (deny beats allow within a source)
    if let Some(deny_arr) = config.get("deny").and_then(|v| v.as_array()) {
        for entry in deny_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.to_string()),
                    command_pattern: None,
                    action: RuleAction::Deny,
                });
            }
        }
    }

    // Allow rules second
    if let Some(allow_arr) = config.get("allow").and_then(|v| v.as_array()) {
        for entry in allow_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.to_string()),
                    command_pattern: None,
                    action: RuleAction::Allow,
                });
            }
        }
    }

    // If ask: true, append a catch-all Ask rule
    if let Some(ask) = config.get("ask").and_then(|v| v.as_bool()) {
        if ask {
            rules.push(PermissionRule {
                tool_pattern: None,
                path_pattern: Some("**".to_string()),
                command_pattern: None,
                action: RuleAction::Ask,
            });
        }
    }

    Ok(rules)
}

/// Parse the `command_gate` config block.
///
/// Structure:
/// ```json
/// {
///   "patterns": {
///     "rm -rf": "ask",
///     "curl *": "deny"
///   }
/// }
/// ```
///
/// Each key is a command substring pattern, each value is the action
/// (`"ask"` or `"deny"`). Rules are generated with no tool pattern
/// (applies to all tools via the command gate).
pub fn parse_command_gate_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    if let Some(patterns) = config.get("patterns").and_then(|v| v.as_object()) {
        for (pattern, action_val) in patterns {
            let action_str = match action_val.as_str() {
                Some(s) => s,
                None => continue,
            };

            let action = match action_str {
                "ask" => RuleAction::Ask,
                "deny" => RuleAction::Deny,
                _ => continue, // unknown action, skip
            };

            rules.push(PermissionRule {
                tool_pattern: None,
                path_pattern: None,
                command_pattern: Some(pattern.clone()),
                action,
            });
        }
    }

    Ok(rules)
}

/// Parse the `file_protection` config block.
///
/// Structure:
/// ```json
/// {
///   "patterns": {
///     ".env*": { "read": "ask", "write": "deny" },
///     "*.pem": { "read": "ask", "write": "deny" }
///   }
/// }
/// ```
///
/// Each key is a file path glob pattern, each value is a map of
/// operation (`"read"` or `"write"`) to action (`"ask"` or `"deny"`).
///
/// Note: In the current PermissionRule model, rules don't carry
/// per-operation filtering. Rules generated here have only a
/// path_pattern set and apply to all operations.
pub fn parse_file_protection_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    if let Some(patterns) = config.get("patterns").and_then(|v| v.as_object()) {
        for (pattern, ops) in patterns {
            let ops_obj = match ops.as_object() {
                Some(o) => o,
                None => continue,
            };

            for (_op, action_val) in ops_obj {
                let action_str = match action_val.as_str() {
                    Some(s) => s,
                    None => continue,
                };

                let action = match action_str {
                    "allow" => RuleAction::Allow,
                    "ask" => RuleAction::Ask,
                    "deny" => RuleAction::Deny,
                    _ => continue,
                };

                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.clone()),
                    command_pattern: None,
                    action,
                });
            }
        }
    }

    Ok(rules)
}

/// Merge multiple rule lists by priority order.
///
/// The slice is expected to be in priority order (first = highest priority).
/// Rules are simply concatenated — during evaluation, the first matching
/// rule wins, so higher-priority sources' rules are checked first.
pub fn merge_rules(rules: &[Vec<PermissionRule>]) -> Vec<PermissionRule> {
    rules.iter().flat_map(|r| r.iter().cloned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuleAction;
    use serde_json::json;
    use std::path::Path;

    // ── Helper ────────────────────────────────────────────────────────────

    fn write_config(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create config dir");
        }
        fs::write(path, content).expect("failed to write config");
    }

    fn make_paths(root: &Path) -> SettingsPaths {
        SettingsPaths {
            hackpi: root.join(".hackpi/guardrails.json"),
            claude_local: root.join(".claude/settings.local.json"),
            claude_project: root.join(".claude/settings.json"),
        }
    }

    // ── load_all: Empty directory ─────────────────────────────────────────

    #[test]
    fn test_load_all_empty_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());
        let rules = load_all(&paths).expect("load_all should succeed");
        assert!(rules.is_empty(), "no config files → empty rules");
    }

    // ── load_all: Non-existent files silently skipped ─────────────────────

    #[test]
    fn test_load_all_skips_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Only create .claude/settings.json
        let settings = dir.path().join(".claude/settings.json");
        write_config(
            &settings,
            r#"{"permissions": {"allow": ["Read(./docs/**)"], "deny": ["Write(./.env)"]}}"#,
        );
        let paths = make_paths(dir.path());
        let rules = load_all(&paths).expect("load_all should succeed");
        assert!(!rules.is_empty(), "should parse the one existing file");
    }

    // ── load_all: Invalid JSON returns error ──────────────────────────────

    #[test]
    fn test_load_all_invalid_json_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hackpi = dir.path().join(".hackpi/guardrails.json");
        write_config(&hackpi, "this is not json");
        let paths = make_paths(dir.path());
        let result = load_all(&paths);
        assert!(result.is_err(), "invalid JSON should return error");
        assert!(
            result.unwrap_err().contains("Invalid JSON"),
            "error should mention Invalid JSON"
        );
    }

    // ── load_hackpi_config: All sections ──────────────────────────────────

    #[test]
    fn test_load_hackpi_config_all_sections() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("guardrails.json");
        write_config(
            &config_path,
            r#"{
                "permissions": {
                    "allow": ["Read(./docs/**)", "Bash(npm run lint)"],
                    "deny": ["Write(./.env)", "Bash(curl *)"]
                },
                "path_access": {
                    "allow": ["~/.config/**", "/tmp/**"],
                    "deny": ["/etc/**"],
                    "ask": true
                },
                "command_gate": {
                    "patterns": {
                        "rm -rf": "ask",
                        "curl *": "deny"
                    }
                },
                "file_protection": {
                    "patterns": {
                        ".env*": { "read": "ask", "write": "deny" },
                        "*.pem": { "read": "ask", "write": "deny" }
                    }
                }
            }"#,
        );

        let rules = load_hackpi_config(&config_path).expect("should parse hackpi config");
        assert!(!rules.is_empty(), "should produce rules from all sections");

        // Check we have at least: 4 permission rules + 3 path_access + 2 command_gate + 4 file_protection
        let deny_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Deny)
            .collect();
        let allow_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Allow)
            .collect();
        let ask_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Ask)
            .collect();

        assert!(!deny_rules.is_empty(), "should have deny rules");
        assert!(!allow_rules.is_empty(), "should have allow rules");
        assert!(!ask_rules.is_empty(), "should have ask rules");

        // Verify specific rules
        let write_env_deny = rules.iter().any(|r| {
            r.action == RuleAction::Deny
                && r.tool_pattern.as_ref().is_some_and(|tp| tp.name == "write")
                && r.path_pattern.as_deref() == Some("./.env")
        });
        assert!(write_env_deny, "should deny Write(./.env)");

        let read_docs_allow = rules.iter().any(|r| {
            r.action == RuleAction::Allow
                && r.tool_pattern.as_ref().is_some_and(|tp| tp.name == "read")
                && r.path_pattern.as_deref() == Some("./docs/**")
        });
        assert!(read_docs_allow, "should allow Read(./docs/**)");

        // Verify command_gate rules
        let rm_ask = rules.iter().any(|r| {
            r.action == RuleAction::Ask
                && r.command_pattern.as_deref() == Some("rm -rf")
                && r.path_pattern.is_none()
        });
        assert!(rm_ask, "should ask for rm -rf");

        // Verify path_access rules
        let etc_deny = rules.iter().any(|r| {
            r.action == RuleAction::Deny
                && r.path_pattern.as_deref() == Some("/etc/**")
                && r.tool_pattern.is_none()
        });
        assert!(etc_deny, "should deny /etc/**");

        // Verify file_protection rules
        let env_ask = rules
            .iter()
            .any(|r| r.action == RuleAction::Ask && r.path_pattern.as_deref() == Some(".env*"));
        assert!(env_ask, "should ask for .env*");

        let env_deny = rules
            .iter()
            .filter(|r| r.action == RuleAction::Deny && r.path_pattern.as_deref() == Some(".env*"));
        assert!(env_deny.count() >= 1, "should deny write for .env*");
    }

    // ── load_all: Claude settings.local.json overrides settings.json ──────
    //
    // The medium-priority source (claude_local) must come before the low-priority
    // source (claude_project) in the merge order so that deny/allow rules from
    // local override project defaults.

    #[test]
    fn test_load_all_claude_local_overrides_claude_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());

        // Claude local (medium priority) — deny Read(foo)
        write_config(
            &paths.claude_local,
            r#"{"permissions": {"deny": ["Read(foo)"]}}"#,
        );

        // Claude project (lowest priority) — allow Read(foo)
        write_config(
            &paths.claude_project,
            r#"{"permissions": {"allow": ["Read(foo)"]}}"#,
        );

        let rules = load_all(&paths).expect("load_all should succeed");
        // claude_local deny should be first (checked before claude_project allow)
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].action,
            RuleAction::Deny,
            "claude local deny should be checked first"
        );
        assert_eq!(
            rules[1].action,
            RuleAction::Allow,
            "claude project allow should be second"
        );
    }

    // ── load_hackpi_config: Missing sections ──────────────────────────────

    #[test]
    fn test_load_hackpi_config_minimal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("guardrails.json");
        write_config(&config_path, r#"{"permissions": {"allow": ["Read(foo)"]}}"#);

        let rules = load_hackpi_config(&config_path).expect("should parse");
        assert_eq!(rules.len(), 1, "one allow rule");
        assert_eq!(rules[0].action, RuleAction::Allow);
    }

    #[test]
    fn test_load_hackpi_config_empty_object() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("guardrails.json");
        write_config(&config_path, r#"{}"#);

        let rules = load_hackpi_config(&config_path).expect("should parse");
        assert!(rules.is_empty(), "empty config → empty rules");
    }

    // ── load_claude_settings ──────────────────────────────────────────────

    #[test]
    fn test_load_claude_settings_with_allow_deny() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.json");
        write_config(
            &settings,
            r#"{"permissions": {"allow": ["Read(./docs/**)"], "deny": ["Write(./.env)"]}}"#,
        );

        let rules = load_claude_settings(&settings).expect("should parse");
        assert_eq!(rules.len(), 2, "one allow + one deny");

        let deny: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Deny)
            .collect();
        let allow: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Allow)
            .collect();
        assert_eq!(deny.len(), 1);
        assert_eq!(allow.len(), 1);
    }

    #[test]
    fn test_load_claude_settings_empty_permissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.json");
        write_config(&settings, r#"{"permissions": {"allow": [], "deny": []}}"#);

        let rules = load_claude_settings(&settings).expect("should parse");
        assert!(rules.is_empty(), "empty arrays → empty rules");
    }

    #[test]
    fn test_load_claude_settings_no_permissions_block() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.json");
        write_config(&settings, r#"{"other": "stuff"}"#);

        let rules = load_claude_settings(&settings).expect("should parse");
        assert!(rules.is_empty(), "no permissions block → empty rules");
    }

    // ── parse_permissions_block ───────────────────────────────────────────

    #[test]
    fn test_parse_permissions_block_valid_entries() {
        let allow = vec![
            "Read(./docs/**)".to_string(),
            "Bash(npm run lint)".to_string(),
        ];
        let deny = vec!["Write(./.env)".to_string()];

        let rules =
            parse_permissions_block(Some(&allow), Some(&deny)).expect("should parse valid entries");

        // deny rules first, then allow
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].action, RuleAction::Deny);
        assert_eq!(rules[1].action, RuleAction::Allow);
        assert_eq!(rules[2].action, RuleAction::Allow);
    }

    #[test]
    fn test_parse_permissions_block_none_allow() {
        let deny = vec!["Write(./.env)".to_string()];

        let rules = parse_permissions_block(None, Some(&deny)).expect("should parse with no allow");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, RuleAction::Deny);
    }

    #[test]
    fn test_parse_permissions_block_none_deny() {
        let allow = vec!["Read(./docs/**)".to_string()];

        let rules = parse_permissions_block(Some(&allow), None).expect("should parse with no deny");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, RuleAction::Allow);
    }

    #[test]
    fn test_parse_permissions_block_malformed_entries_skipped() {
        let allow = vec![
            "Read(./docs/**)".to_string(),
            "".to_string(),                  // empty — skipped
            "InvalidTool(foo)".to_string(),  // unknown tool — skipped
            "./bare-pattern.rs".to_string(), // bare pattern — valid
        ];
        let deny = vec![
            "()".to_string(), // no tool name — skipped
        ];

        let rules = parse_permissions_block(Some(&allow), Some(&deny))
            .expect("should skip malformed entries");
        // deny is empty (only malformed), allow should give 2 valid rules
        // bare pattern with no tool prefix → applies to all tools
        assert_eq!(rules.len(), 2, "only valid entries create rules");
    }

    #[test]
    fn test_parse_permissions_block_bare_pattern_no_tool() {
        let allow = vec!["./some-file".to_string()];
        let rules = parse_permissions_block(Some(&allow), None).expect("should parse");
        assert_eq!(rules.len(), 1);
        assert!(
            rules[0].tool_pattern.is_none(),
            "bare pattern → no tool filter"
        );
        assert!(
            rules[0].path_pattern.is_some(),
            "bare pattern → path_pattern set"
        );
    }

    // ── parse_path_access_block ───────────────────────────────────────────

    #[test]
    fn test_parse_path_access_block_allow_deny() {
        let config = json!({
            "allow": ["~/.config/**", "/tmp/**"],
            "deny": ["/etc/**"]
        });

        let rules = parse_path_access_block(&config).expect("should parse");
        // deny first, then allow
        assert_eq!(rules.len(), 3);

        assert_eq!(rules[0].action, RuleAction::Deny);
        assert_eq!(rules[0].path_pattern.as_deref(), Some("/etc/**"));

        assert_eq!(rules[1].action, RuleAction::Allow);
        assert!(rules[1].path_pattern.as_deref() == Some("~/.config/**"));

        assert_eq!(rules[2].action, RuleAction::Allow);
        assert!(rules[2].path_pattern.as_deref() == Some("/tmp/**"));

        // All should have no tool pattern (applies to all tools)
        for rule in &rules {
            assert!(
                rule.tool_pattern.is_none(),
                "path_access rules apply to all tools"
            );
        }
    }

    #[test]
    fn test_parse_path_access_block_ask_true_generates_ask_rule() {
        let config = json!({
            "allow": ["/safe/**"],
            "deny": ["/danger/**"],
            "ask": true
        });

        let rules = parse_path_access_block(&config).expect("should parse");
        // deny, allow, ask catch-all
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[2].action, RuleAction::Ask);
        assert_eq!(
            rules[2].path_pattern.as_deref(),
            Some("**"),
            "catch-all pattern should be **"
        );
    }

    #[test]
    fn test_parse_path_access_block_ask_false_no_ask_rule() {
        let config = json!({
            "allow": ["/safe/**"],
            "deny": ["/danger/**"],
            "ask": false
        });

        let rules = parse_path_access_block(&config).expect("should parse");
        assert_eq!(rules.len(), 2, "no ask rule when ask: false");
        assert!(rules.iter().all(|r| r.action != RuleAction::Ask));
    }

    #[test]
    fn test_parse_path_access_block_no_ask_field() {
        let config = json!({
            "allow": ["/safe/**"]
        });

        let rules = parse_path_access_block(&config).expect("should parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, RuleAction::Allow);
    }

    // ── parse_command_gate_block ──────────────────────────────────────────

    #[test]
    fn test_parse_command_gate_block_patterns() {
        let config = json!({
            "patterns": {
                "rm -rf": "ask",
                "curl *": "deny"
            }
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert_eq!(rules.len(), 2);

        let rm_ask = rules
            .iter()
            .find(|r| r.command_pattern.as_deref() == Some("rm -rf"));
        assert!(rm_ask.is_some());
        assert_eq!(rm_ask.unwrap().action, RuleAction::Ask);

        let curl_deny = rules
            .iter()
            .find(|r| r.command_pattern.as_deref() == Some("curl *"));
        assert!(curl_deny.is_some());
        assert_eq!(curl_deny.unwrap().action, RuleAction::Deny);
    }

    #[test]
    fn test_parse_command_gate_block_empty_patterns() {
        let config = json!({"patterns": {}});

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert!(rules.is_empty());
    }

    #[test]
    fn test_parse_command_gate_block_unknown_action_skipped() {
        let config = json!({
            "patterns": {
                "rm -rf": "ask",
                "sudo *": "maybe" // unknown action
            }
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert_eq!(rules.len(), 1, "unknown action entry skipped");
    }

    // ── parse_file_protection_block ───────────────────────────────────────

    #[test]
    fn test_parse_file_protection_block_patterns() {
        let config = json!({
            "patterns": {
                ".env*": { "read": "ask", "write": "deny" },
                "*.pem": { "read": "ask", "write": "deny" }
            }
        });

        let rules = parse_file_protection_block(&config).expect("should parse");
        // 2 patterns × 2 operations each = 4 rules
        assert_eq!(rules.len(), 4);

        let env_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.path_pattern.as_deref() == Some(".env*"))
            .collect();
        assert_eq!(env_rules.len(), 2, ".env* should have read+write rules");

        let pem_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.path_pattern.as_deref() == Some("*.pem"))
            .collect();
        assert_eq!(pem_rules.len(), 2, "*.pem should have read+write rules");
    }

    #[test]
    fn test_parse_file_protection_block_single_op() {
        let config = json!({
            "patterns": {
                ".secret": { "read": "deny" }
            }
        });

        let rules = parse_file_protection_block(&config).expect("should parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, RuleAction::Deny);
        assert_eq!(rules[0].path_pattern.as_deref(), Some(".secret"));
    }

    #[test]
    fn test_parse_file_protection_block_empty() {
        let config = json!({"patterns": {}});
        let rules = parse_file_protection_block(&config).expect("should parse");
        assert!(rules.is_empty());
    }

    #[test]
    fn test_parse_file_protection_block_unknown_action_skipped() {
        let config = json!({
            "patterns": {
                ".env": { "read": "allow", "write": "maybe" }
            }
        });
        let rules = parse_file_protection_block(&config).expect("should parse");
        // "write" action "maybe" is unknown → skipped, only "read" rule remains
        assert_eq!(rules.len(), 1, "unknown action entry should be skipped");
        assert_eq!(rules[0].action, RuleAction::Allow);
        assert_eq!(rules[0].path_pattern.as_deref(), Some(".env"));
    }

    #[test]
    fn test_parse_file_protection_block_invalid_operation_value_skipped() {
        let config = json!({
            "patterns": {
                ".env": { "read": "ask", "write": 123 }
            }
        });
        let rules = parse_file_protection_block(&config).expect("should parse");
        // "write" value is a number, not a string → skipped
        assert_eq!(
            rules.len(),
            1,
            "non-string operation action should be skipped"
        );
        assert_eq!(rules[0].path_pattern.as_deref(), Some(".env"));
    }

    #[test]
    fn test_parse_file_protection_block_string_instead_of_object_skipped() {
        let config = json!({
            "patterns": {
                ".env": "just a string, not an object"
            }
        });
        let rules = parse_file_protection_block(&config).expect("should parse");
        assert!(
            rules.is_empty(),
            "non-object pattern value should be skipped"
        );
    }

    /// Test that parse_file_protection_block with a pattern that has "allow" for read
    /// and "deny" for write produces the correct per-operation rules.
    #[test]
    fn test_parse_file_protection_block_per_op_rules() {
        let config = json!({
            "patterns": {
                ".secret": { "read": "allow", "write": "deny" }
            }
        });
        let rules = parse_file_protection_block(&config).expect("should parse");
        assert_eq!(rules.len(), 2, "should produce 2 per-operation rules");

        let allow_rule = rules.iter().find(|r| r.action == RuleAction::Allow);
        let deny_rule = rules.iter().find(|r| r.action == RuleAction::Deny);
        assert!(allow_rule.is_some(), "should have an allow rule");
        assert!(deny_rule.is_some(), "should have a deny rule");
        for rule in &rules {
            assert_eq!(
                rule.path_pattern.as_deref(),
                Some(".secret"),
                "all rules should have the same path pattern"
            );
        }
    }

    // ── merge_rules ───────────────────────────────────────────────────────

    #[test]
    fn test_merge_rules_concatenates_in_order() {
        let source1 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/high-priority-deny".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let source2 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/low-priority-allow".to_string()),
            command_pattern: None,
            action: RuleAction::Allow,
        }];

        let merged = merge_rules(&[source1, source2]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].action, RuleAction::Deny);
        assert_eq!(merged[1].action, RuleAction::Allow);
    }

    #[test]
    fn test_merge_rules_empty_sources() {
        let merged = merge_rules(&[] as &[Vec<PermissionRule>]);
        assert!(merged.is_empty());
    }

    #[test]
    fn test_merge_rules_three_sources() {
        let s1 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Deny,
        }];
        let s2 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        }];
        let s3 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Ask,
        }];

        let merged = merge_rules(&[s1, s2, s3]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].action, RuleAction::Deny);
        assert_eq!(merged[1].action, RuleAction::Allow);
        assert_eq!(merged[2].action, RuleAction::Ask);
    }

    // ── load_all: Merging priority ────────────────────────────────────────

    #[test]
    fn test_load_all_higher_priority_overrides_lower() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());

        // Hackpi (highest priority) — deny Read(foo)
        write_config(&paths.hackpi, r#"{"permissions": {"deny": ["Read(foo)"]}}"#);

        // Claude local (medium priority) — allow Read(foo)
        write_config(
            &paths.claude_local,
            r#"{"permissions": {"allow": ["Read(foo)"]}}"#,
        );

        // Claude project (lowest priority) — also allow
        write_config(
            &paths.claude_project,
            r#"{"permissions": {"allow": ["Read(foo)"]}}"#,
        );

        let rules = load_all(&paths).expect("load_all should succeed");
        // Order: hackpi deny, claude_local allow, claude_project allow
        // deny is first → gets checked first → higher priority wins
        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules[0].action,
            RuleAction::Deny,
            "hackpi deny should be first"
        );
    }

    // ── Error handling ────────────────────────────────────────────────────

    #[test]
    fn test_load_hackpi_config_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("bad.json");
        write_config(&config_path, "{invalid json}");

        let result = load_hackpi_config(&config_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_load_hackpi_config_file_not_found() {
        let result = load_hackpi_config(Path::new("/nonexistent/file.json"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read"));
    }

    #[test]
    fn test_load_claude_settings_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = dir.path().join("settings.json");
        write_config(&settings, "not json");

        let result = load_claude_settings(&settings);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }
}
