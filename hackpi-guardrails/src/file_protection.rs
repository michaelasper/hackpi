use crate::{FileOp, GuardReason, GuardResult, GuardType, PermissionRule, RuleAction};
use std::path::Path;

// ── Types ────────────────────────────────────────────────────────────────────

/// A protected file pattern with per-operation actions.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectedPattern {
    /// Glob pattern for matching file paths.
    ///
    /// Patterns without a `/` match the filename at any directory depth
    /// (similar to `.gitignore` semantics).
    /// Patterns with `**/` or `/` match exactly as specified.
    pub pattern: &'static str,
    /// Action to take on read operations.
    pub read_action: RuleAction,
    /// Action to take on write operations.
    pub write_action: RuleAction,
}

// ── Default Protected Patterns ──────────────────────────────────────────────

/// Default protected file patterns.
///
/// These are checked after any user-configured rules and serve as
/// a safety net for commonly sensitive files. Patterns without a `/`
/// match the filename at any directory depth.
pub static DEFAULT_PROTECTED_PATTERNS: &[ProtectedPattern] = &[
    ProtectedPattern {
        pattern: ".env",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: ".env.*",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "**/credentials*",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "**/secrets/**",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "**/.git/**",
        read_action: RuleAction::Allow,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "*.pem",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "*.key",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "id_rsa",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
    ProtectedPattern {
        pattern: "id_ed25519",
        read_action: RuleAction::Ask,
        write_action: RuleAction::Deny,
    },
];

// ── Public API ───────────────────────────────────────────────────────────────

/// Main entry: check a path + operation against protected file patterns.
///
/// Evaluation order:
/// 1. Check configured rules (from config files) — Deny beats Allow
/// 2. Check built-in [`DEFAULT_PROTECTED_PATTERNS`] as fallback
/// 3. No matches → [`GuardResult::Allow`]
pub fn check(path: &Path, op: &FileOp, rules: &[PermissionRule], tool: &str) -> GuardResult {
    // 1. Check configured rules first
    if let Some(result) = check_file_rules(path, op, rules, tool) {
        return result;
    }

    // 2. Check built-in defaults
    if let Some(result) = check_against_defaults(path, op) {
        return result;
    }

    // 3. No matches → Allow
    GuardResult::Allow
}

/// Match path against configured rules with tool scoping.
///
/// Iterates rules in order, checking:
/// - Tool match via [`crate::pattern::rule_matches_tool`]
/// - Operation match via [`crate::pattern::rule_matches_operation`]
/// - Path glob match
///
/// Returns `Deny` for matching deny rules, `Allow` for matching allow rules,
/// `Ask` for matching ask rules, or `None` if no rule matches.
pub fn check_file_rules(
    path: &Path,
    op: &FileOp,
    rules: &[PermissionRule],
    tool: &str,
) -> Option<GuardResult> {
    for rule in rules {
        // Check tool match
        if !crate::pattern::rule_matches_tool(rule, tool) {
            continue;
        }

        // Check operation match (path-only rules or combined rules)
        if !crate::pattern::rule_matches_operation(rule, op) {
            continue;
        }

        // Must have a path pattern to match against
        let path_pattern = match &rule.path_pattern {
            Some(p) => p,
            None => continue,
        };

        // Compile and check path glob
        let matcher = crate::pattern::compile_glob(path_pattern).ok()?;
        if !matcher.is_match(path) {
            continue;
        }

        // Rule matches — return the corresponding GuardResult
        return Some(match rule.action {
            RuleAction::Deny => GuardResult::Deny(format!(
                "Access denied to '{}' by file protection rule '{}'",
                path.display(),
                path_pattern
            )),
            RuleAction::Allow => GuardResult::Allow,
            RuleAction::Ask => GuardResult::Ask(GuardReason {
                guard: GuardType::FileProtection,
                tool: tool.to_string(),
                details: format!(
                    "Access to '{}' matches protected file pattern '{}'",
                    path.display(),
                    path_pattern
                ),
            }),
        });
    }

    None
}

/// Match path against [`DEFAULT_PROTECTED_PATTERNS`].
///
/// For patterns without a `/`, also tries matching with a `**/` prefix
/// to support matching at any directory depth (like `.gitignore` semantics).
///
/// Returns the appropriate [`GuardResult`] for the matched operation,
/// or `None` if no pattern matches.
pub fn check_against_defaults(path: &Path, op: &FileOp) -> Option<GuardResult> {
    for pp in DEFAULT_PROTECTED_PATTERNS {
        if let Some(true) = check_pattern_against(path, pp) {
            let action = match op {
                FileOp::Read => pp.read_action.clone(),
                FileOp::Write => pp.write_action.clone(),
            };
            return Some(result_for_action(action, path, pp.pattern, op));
        }
    }

    None
}

// ── Internal Helpers ─────────────────────────────────────────────────────────

/// Check if a path matches a protected pattern (trying bare and `**/`-prefixed).
///
/// Returns `Some(true)` if it matches, `Some(false)` if it doesn't, or `None`
/// if the pattern is invalid.
fn check_pattern_against(path: &Path, pp: &ProtectedPattern) -> Option<bool> {
    // Try the pattern as-is first
    if let Ok(matcher) = crate::pattern::compile_glob(pp.pattern) {
        if matcher.is_match(path) {
            return Some(true);
        }
    }

    // For patterns without `/`, also try with `**/` prefix for depth matching
    if !pp.pattern.contains('/') {
        let prefixed = format!("**/{}", pp.pattern);
        if let Ok(matcher) = crate::pattern::compile_glob(&prefixed) {
            if matcher.is_match(path) {
                return Some(true);
            }
        }
    }

    Some(false)
}

/// Convert a matched pattern's action + operation into a [`GuardResult`].
fn result_for_action(action: RuleAction, path: &Path, pattern: &str, op: &FileOp) -> GuardResult {
    let op_str = match op {
        FileOp::Read => "Reading",
        FileOp::Write => "Writing",
    };

    match action {
        RuleAction::Allow => GuardResult::Allow,
        RuleAction::Ask => GuardResult::Ask(GuardReason {
            guard: GuardType::FileProtection,
            tool: String::new(),
            details: format!(
                "{} '{}' matches protected pattern '{}'",
                op_str,
                path.display(),
                pattern
            ),
        }),
        RuleAction::Deny => GuardResult::Deny(format!(
            "{} '{}' denied by protected pattern '{}'",
            op_str,
            path.display(),
            pattern
        )),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileOp, PermissionRule, RuleAction, ToolPattern};
    use std::path::Path;

    // ── check_against_defaults ─────────────────────────────────────────────

    #[test]
    fn test_read_env_returns_ask() {
        let path = Path::new(".env");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_write_env_returns_deny() {
        let path = Path::new(".env");
        let result = check_against_defaults(path, &FileOp::Write);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_read_secrets_key_pem_returns_ask() {
        let path = Path::new("secrets/key.pem");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_write_git_config_returns_deny() {
        let path = Path::new(".git/config");
        let result = check_against_defaults(path, &FileOp::Write);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_read_git_config_returns_allow() {
        let path = Path::new(".git/config");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), GuardResult::Allow);
    }

    #[test]
    fn test_read_src_main_rs_returns_none() {
        let path = Path::new("src/main.rs");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_none(), "non-protected file should not match");
    }

    #[test]
    fn test_read_nested_env_returns_ask() {
        let path = Path::new("src/.env");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "nested .env should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_write_nested_env_returns_deny() {
        let path = Path::new("config/production/.env");
        let result = check_against_defaults(path, &FileOp::Write);
        assert!(result.is_some(), "deeply nested .env should match");
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_read_env_local_returns_ask() {
        let path = Path::new(".env.local");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_read_credentials_file_returns_ask() {
        let path = Path::new("config/credentials.json");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_read_secrets_in_subdir_returns_ask() {
        let path = Path::new("secrets/production/db_pass.txt");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_read_key_file_returns_ask() {
        let path = Path::new("ssh/key.pem");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_read_rsa_key_returns_ask() {
        let path = Path::new("id_rsa");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_read_ed25519_key_returns_ask() {
        let path = Path::new("id_ed25519");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_write_ed25519_key_returns_deny() {
        let path = Path::new("id_ed25519");
        let result = check_against_defaults(path, &FileOp::Write);
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_key_file_in_deep_path_returns_ask() {
        let path = Path::new("home/user/.ssh/id_rsa");
        let result = check_against_defaults(path, &FileOp::Read);
        // id_rsa without `/` should also match at depth (via `**/id_rsa`)
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    // ── check_file_rules ────────────────────────────────────────────────────

    #[test]
    fn test_file_rules_deny_matches() {
        let path = Path::new("my.env");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];

        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_file_rules_allow_matches() {
        let path = Path::new("my.env");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        }];

        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), GuardResult::Allow);
    }

    #[test]
    fn test_file_rules_ask_matches() {
        let path = Path::new("my.env");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Ask,
        }];

        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_file_rules_tool_scoped_no_match() {
        let path = Path::new("my.env");
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "write".into(),
                pattern: "*.env".into(),
            }),
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];

        // Rule is scoped to "write", checking with "read" should not match
        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_none());
    }

    #[test]
    fn test_file_rules_tool_scoped_matches() {
        let path = Path::new("my.env");
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "*.env".into(),
            }),
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];

        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_some());
        assert!(matches!(result.unwrap(), GuardResult::Deny(_)));
    }

    #[test]
    fn test_file_rules_no_path_pattern_skipped() {
        let path = Path::new("file.txt");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("rm".into()),
            operation: None,
            action: RuleAction::Deny,
        }];

        // Command-only rule should be skipped
        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_none());
    }

    #[test]
    fn test_file_rules_no_path_match_returns_none() {
        let path = Path::new("safe.txt");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];

        let result = check_file_rules(path, &FileOp::Read, &rules, "read");
        assert!(result.is_none());
    }

    // ── check (full pipeline) ────────────────────────────────────────────────

    #[test]
    fn test_check_non_protected_path_returns_allow() {
        let path = Path::new("src/main.rs");
        let result = check(path, &FileOp::Read, &[], "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_env_read_returns_ask() {
        let path = Path::new(".env");
        let result = check(path, &FileOp::Read, &[], "read");
        assert!(matches!(result, GuardResult::Ask(_)));
    }

    #[test]
    fn test_check_env_write_returns_deny() {
        let path = Path::new(".env");
        let result = check(path, &FileOp::Write, &[], "write");
        assert!(matches!(result, GuardResult::Deny(_)));
    }

    #[test]
    fn test_check_git_config_read_returns_allow() {
        let path = Path::new(".git/config");
        let result = check(path, &FileOp::Read, &[], "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_git_config_write_returns_deny() {
        let path = Path::new(".git/config");
        let result = check(path, &FileOp::Write, &[], "write");
        assert!(matches!(result, GuardResult::Deny(_)));
    }

    #[test]
    fn test_check_secrets_key_pem_read_returns_ask() {
        let path = Path::new("secrets/key.pem");
        let result = check(path, &FileOp::Read, &[], "read");
        assert!(matches!(result, GuardResult::Ask(_)));
    }

    #[test]
    fn test_check_custom_rule_overrides_default() {
        let path = Path::new(".env");
        // Custom allow rule should override default Ask
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some(".env".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        }];

        let result = check(path, &FileOp::Read, &rules, "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_custom_deny_rule_blocks_non_protected() {
        let path = Path::new("src/main.rs");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.rs".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];

        let result = check(path, &FileOp::Read, &rules, "read");
        assert!(matches!(result, GuardResult::Deny(_)));
    }

    #[test]
    fn test_check_custom_ask_rule_overrides_default_allow() {
        let path = Path::new(".git/config");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/.git/**".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Ask,
        }];

        // Custom Ask rule should override default Allow
        let result = check(path, &FileOp::Read, &rules, "read");
        assert!(matches!(result, GuardResult::Ask(_)));
    }

    // ── ProtectedPattern struct ──────────────────────────────────────────────

    #[test]
    fn test_default_protected_patterns_count() {
        assert_eq!(DEFAULT_PROTECTED_PATTERNS.len(), 9);
    }

    #[test]
    fn test_default_pattern_env_read_action() {
        let pp = DEFAULT_PROTECTED_PATTERNS
            .iter()
            .find(|p| p.pattern == ".env")
            .expect(".env should be in defaults");
        assert_eq!(pp.read_action, RuleAction::Ask);
        assert_eq!(pp.write_action, RuleAction::Deny);
    }

    #[test]
    fn test_default_pattern_git_read_action() {
        let pp = DEFAULT_PROTECTED_PATTERNS
            .iter()
            .find(|p| p.pattern == "**/.git/**")
            .expect("**/.git/** should be in defaults");
        assert_eq!(pp.read_action, RuleAction::Allow);
        assert_eq!(pp.write_action, RuleAction::Deny);
    }

    // ── Same file, different ops ────────────────────────────────────────────

    #[test]
    fn test_same_file_read_asks_write_denies() {
        // For .env: read → Ask, write → Deny
        let path = Path::new(".env");
        let read_result = check(path, &FileOp::Read, &[], "read");
        assert!(
            matches!(read_result, GuardResult::Ask(_)),
            "read .env should ask"
        );

        let write_result = check(path, &FileOp::Write, &[], "write");
        assert!(
            matches!(write_result, GuardResult::Deny(_)),
            "write .env should deny"
        );
    }

    #[test]
    fn test_same_file_git_config_read_allow_write_deny() {
        let path = Path::new(".git/config");
        let read_result = check(path, &FileOp::Read, &[], "read");
        assert_eq!(
            read_result,
            GuardResult::Allow,
            "read .git/config should allow"
        );

        let write_result = check(path, &FileOp::Write, &[], "write");
        assert!(
            matches!(write_result, GuardResult::Deny(_)),
            "write .git/config should deny"
        );
    }

    #[test]
    fn test_same_file_protected_both_ops_with_custom_rules() {
        let path = Path::new(".env");
        // Custom rules: allow read, ask write
        let rules = vec![
            PermissionRule {
                tool_pattern: None,
                path_pattern: Some(".env".into()),
                command_pattern: None,
                operation: None,
                action: RuleAction::Allow,
            },
            PermissionRule {
                tool_pattern: None,
                path_pattern: Some(".env".into()),
                command_pattern: None,
                operation: None,
                action: RuleAction::Ask,
            },
        ];

        // Allow rule should win (it's first in the list)
        let read_result = check(path, &FileOp::Read, &rules, "read");
        assert_eq!(
            read_result,
            GuardResult::Allow,
            "custom allow should override default ask"
        );

        let write_result = check(path, &FileOp::Write, &rules, "write");
        assert_eq!(
            write_result,
            GuardResult::Allow,
            "first matching rule wins for write too"
        );
    }

    // ── Nested subdirectory matching ─────────────────────────────────────────

    #[test]
    fn test_nested_deeply_env_local() {
        let path = Path::new("a/very/deeply/nested/folder/.env.local");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "deeply nested .env.local should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_nested_credentials_in_subdir() {
        let path = Path::new("config/backups/credentials.old.json");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "nested credentials file should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_nested_secrets_deeply() {
        let path = Path::new("infra/secrets/production/database/password.txt");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "deeply nested secrets file should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_nested_pem_in_deep_path() {
        let path = Path::new("certs/2024/wildcard.example.com.pem");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "nested .pem should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_nested_key_in_deep_path() {
        let path = Path::new("ssh/keys/deploy.key");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_some(), "nested .key should match");
        assert!(matches!(result.unwrap(), GuardResult::Ask(_)));
    }

    #[test]
    fn test_nested_not_protected_returns_none() {
        let path = Path::new("src/components/Button.tsx");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(result.is_none(), "non-protected file should not match");
    }

    #[test]
    fn test_nested_git_in_inside_hidden_dir() {
        let path = Path::new(".git/objects/pack/pack-abc123.pack");
        let result = check_against_defaults(path, &FileOp::Read);
        assert!(
            matches!(result, Some(GuardResult::Allow)),
            "reading nested .git objects should allow"
        );

        let write_result = check_against_defaults(path, &FileOp::Write);
        assert!(
            matches!(write_result, Some(GuardResult::Deny(_))),
            "writing nested .git objects should deny"
        );
    }

    /// Test that a custom rule for a nested path pattern works correctly.
    #[test]
    fn test_nested_custom_rule_matches_subdirectory() {
        let path = Path::new("logs/debug.log");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("logs/**".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];
        let result = check(path, &FileOp::Read, &rules, "read");
        assert!(
            matches!(result, GuardResult::Deny(_)),
            "custom rule for logs/ should deny"
        );
    }

    #[test]
    fn test_nested_custom_rule_with_globstar() {
        let path = Path::new("node_modules/express/lib/index.js");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/node_modules/**".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Deny,
        }];
        let result = check(path, &FileOp::Read, &rules, "read");
        assert!(
            matches!(result, GuardResult::Deny(_)),
            "globstar pattern should match nested node_modules"
        );
    }
}
