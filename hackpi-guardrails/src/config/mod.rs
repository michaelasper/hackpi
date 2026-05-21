pub mod claude_code;
pub mod hot_reload;
pub mod loading;
pub mod parsing;
pub mod structs;
pub mod validation;

pub use loading::*;
pub use parsing::*;
pub use structs::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionRule, RuleAction, SettingsPaths};
    use serde_json::json;
    use std::fs;
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

    #[test]
    fn test_load_all_partial_failure_recovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = make_paths(dir.path());

        // Valid hackpi config
        write_config(
            &paths.hackpi,
            r#"{"permissions": {"allow": ["Read(./docs/**)"]}}"#,
        );
        // Invalid claude local config
        write_config(&paths.claude_local, "not valid json");
        // Valid claude project config
        write_config(
            &paths.claude_project,
            r#"{"permissions": {"deny": ["Write(./.env)"]}}"#,
        );

        // Should succeed with partial results — rules from hackpi and claude_project
        let rules = load_all(&paths).expect("should return partial results");
        assert!(!rules.is_empty(), "should have rules from valid files");

        let allow_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Allow)
            .collect();
        let deny_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Deny)
            .collect();
        assert!(
            !allow_rules.is_empty(),
            "should have allow rule from hackpi config"
        );
        assert!(
            !deny_rules.is_empty(),
            "should have deny rule from claude_project config"
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
    fn test_parse_permissions_block_rejects_empty_entry() {
        let allow = vec!["".to_string()];
        let result = parse_permissions_block(Some(&allow), None);
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("empty entry should be rejected"),
        };
        assert!(err.contains("empty"), "error should mention 'empty': {err}");
    }

    #[test]
    fn test_parse_permissions_block_rejects_unknown_tool() {
        let allow = vec!["InvalidTool(foo)".to_string()];
        let result = parse_permissions_block(Some(&allow), None);
        assert!(result.is_err(), "unknown tool should be rejected");
        assert!(
            result.unwrap_err().contains("unknown tool"),
            "error should mention unknown tool"
        );
    }

    #[test]
    fn test_parse_permissions_block_rejects_empty_tool_name() {
        let allow = vec!["()".to_string()];
        let result = parse_permissions_block(Some(&allow), None);
        assert!(result.is_err(), "empty tool name should be rejected");
        assert!(
            result.unwrap_err().contains("empty tool name"),
            "error should mention empty tool name"
        );
    }

    #[test]
    fn test_parse_permissions_block_rejects_missing_closing_paren() {
        let allow = vec!["Read(./docs/**".to_string()];
        let result = parse_permissions_block(Some(&allow), None);
        assert!(result.is_err(), "missing closing paren should be rejected");
        assert!(
            result.unwrap_err().contains("missing closing"),
            "error should mention missing closing paren"
        );
    }

    #[test]
    fn test_parse_permissions_block_rejects_empty_pattern() {
        let allow = vec!["Read()".to_string()];
        let result = parse_permissions_block(Some(&allow), None);
        assert!(result.is_err(), "empty pattern should be rejected");
        assert!(
            result.unwrap_err().contains("empty pattern"),
            "error should mention empty pattern"
        );
    }

    #[test]
    fn test_parse_permissions_block_bare_pattern_is_valid() {
        // Bare patterns (no parens) are valid — they apply to all tools
        let allow = vec!["./bare-pattern.rs".to_string()];
        let rules =
            parse_permissions_block(Some(&allow), None).expect("bare patterns should be valid");
        assert_eq!(rules.len(), 1, "bare pattern should produce a rule");
        assert!(
            rules[0].tool_pattern.is_none(),
            "bare pattern → no tool filter"
        );
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
    fn test_parse_command_gate_block_unknown_action_rejected() {
        let config = json!({
            "patterns": {
                "rm -rf": "ask",
                "sudo *": "maybe" // unknown action
            }
        });

        let result = parse_command_gate_block(&config);
        assert!(result.is_err(), "unknown action should be rejected");
        assert!(
            result.unwrap_err().contains("unknown action"),
            "error should mention unknown action"
        );
    }

    #[test]
    fn test_parse_command_gate_block_non_string_action_rejected() {
        let config = json!({
            "patterns": {
                "rm -rf": 123 // number instead of string
            }
        });

        let result = parse_command_gate_block(&config);
        assert!(result.is_err(), "non-string action should be rejected");
    }

    // ── parse_command_gate_block with allow/deny/ask arrays ────────────────

    #[test]
    fn test_parse_command_gate_block_deny_array() {
        let config = json!({
            "deny": ["git *", "gh *"]
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert_eq!(rules.len(), 2);

        let git_deny = rules
            .iter()
            .find(|r| r.command_pattern.as_deref() == Some("git *"));
        assert!(git_deny.is_some());
        assert_eq!(git_deny.unwrap().action, RuleAction::Deny);

        let gh_deny = rules
            .iter()
            .find(|r| r.command_pattern.as_deref() == Some("gh *"));
        assert!(gh_deny.is_some());
        assert_eq!(gh_deny.unwrap().action, RuleAction::Deny);
    }

    #[test]
    fn test_parse_command_gate_block_allow_array() {
        let config = json!({
            "allow": ["git status", "git log"]
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert_eq!(rules.len(), 2);

        for rule in &rules {
            assert_eq!(rule.action, RuleAction::Allow);
        }
    }

    #[test]
    fn test_parse_command_gate_block_ask_array() {
        let config = json!({
            "ask": ["npm *"]
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].action, RuleAction::Ask);
        assert_eq!(rules[0].command_pattern.as_deref(), Some("npm *"));
    }

    #[test]
    fn test_parse_command_gate_block_deny_before_allow_order() {
        let config = json!({
            "allow": ["git status"],
            "deny": ["git *"]
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        // deny rules come first, then allow
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].action, RuleAction::Deny);
        assert_eq!(rules[1].action, RuleAction::Allow);
    }

    #[test]
    fn test_parse_command_gate_block_combined_patterns_and_arrays() {
        let config = json!({
            "patterns": {
                "rm -rf": "ask"
            },
            "deny": ["git *"],
            "allow": ["git status"]
        });

        let rules = parse_command_gate_block(&config).expect("should parse");
        // deny rules first, then patterns, then allow
        assert!(rules.len() >= 3);

        let deny_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Deny)
            .collect();
        let ask_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Ask)
            .collect();
        let allow_rules: Vec<_> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Allow)
            .collect();

        assert!(!deny_rules.is_empty());
        assert!(!ask_rules.is_empty());
        assert!(!allow_rules.is_empty());
    }

    // ── parse_command_gate_extras (allow_git_in_bash) ────────────────────

    #[test]
    fn test_parse_command_gate_extras_allow_git_in_bash_true() {
        let config = json!({
            "allow_git_in_bash": true,
            "deny": ["git *"]
        });

        let result = parse_command_gate_extras(&config);
        assert!(result.allow_git_in_bash);
    }

    #[test]
    fn test_parse_command_gate_extras_allow_git_in_bash_false() {
        let config = json!({
            "allow_git_in_bash": false,
            "deny": ["git *"]
        });

        let result = parse_command_gate_extras(&config);
        assert!(!result.allow_git_in_bash);
    }

    #[test]
    fn test_parse_command_gate_extras_default_false() {
        let config = json!({
            "deny": ["git *"]
        });

        let result = parse_command_gate_extras(&config);
        assert!(!result.allow_git_in_bash);
    }

    // ── vcs_bypass_rules ────────────────────────────────────────────────

    #[test]
    fn test_vcs_bypass_rules_returns_git_and_gh_allow() {
        let rules = vcs_bypass_rules();
        assert_eq!(rules.len(), 2);

        assert_eq!(rules[0].action, RuleAction::Allow);
        assert_eq!(rules[0].command_pattern.as_deref(), Some("git"));

        assert_eq!(rules[1].action, RuleAction::Allow);
        assert_eq!(rules[1].command_pattern.as_deref(), Some("gh"));
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
    fn test_parse_file_protection_block_unknown_action_rejected() {
        let config = json!({
            "patterns": {
                ".env": { "read": "allow", "write": "maybe" }
            }
        });
        let result = parse_file_protection_block(&config);
        assert!(result.is_err(), "unknown action should be rejected");
        assert!(
            result.unwrap_err().contains("unknown action"),
            "error should mention unknown action"
        );
    }

    #[test]
    fn test_parse_file_protection_block_invalid_operation_value_rejected() {
        let config = json!({
            "patterns": {
                ".env": { "read": "ask", "write": 123 }
            }
        });
        let result = parse_file_protection_block(&config);
        assert!(
            result.is_err(),
            "non-string action value should be rejected"
        );
        assert!(
            result.unwrap_err().contains("expected a string action"),
            "error should mention expected string"
        );
    }

    #[test]
    fn test_parse_file_protection_block_string_instead_of_object_rejected() {
        let config = json!({
            "patterns": {
                ".env": "just a string, not an object"
            }
        });
        let result = parse_file_protection_block(&config);
        assert!(
            result.is_err(),
            "non-object pattern value should be rejected"
        );
        assert!(
            result.unwrap_err().contains("expected an object"),
            "error should mention expected object"
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
            operation: None,
            action: RuleAction::Deny,
        }];
        let source2 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/low-priority-allow".to_string()),
            command_pattern: None,
            operation: None,
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
            operation: None,
            action: RuleAction::Deny,
        }];
        let s2 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        }];
        let s3 = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            operation: None,
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
