use crate::{FileOp, PermissionRule, ToolPattern};
use globset::{Glob, GlobMatcher};
use std::path::Path;

/// Known tool names recognized by the guard system.
const KNOWN_TOOLS: &[&str] = &["bash", "read", "write", "edit", "search_grep", "searchgrep"];

/// Check if a tool name is one of the known tools.
///
/// Comparison is case-insensitive.
pub fn is_known_tool(name: &str) -> bool {
    KNOWN_TOOLS.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// Parse a permission string in the format `ToolName(pattern)` (Claude Code style).
///
/// The tool name portion is case-insensitive. If no tool prefix is detected
/// (i.e., the string does not contain parentheses), returns the entire string
/// as a bare pattern with no tool restriction.
///
/// Returns `None` when:
/// - The string is empty
/// - The format has parentheses but the tool name is empty
/// - The parentheses are present but the inner pattern is empty
/// - The tool name before the parentheses is not a known tool
pub fn parse_permission_string(s: &str) -> Option<(Option<ToolPattern>, String)> {
    if s.is_empty() {
        return None;
    }

    // Check for ToolName(pattern) format
    if let Some(open_idx) = s.find('(') {
        let tool_name = &s[..open_idx];
        if tool_name.is_empty() {
            return None;
        }

        // Must have closing paren at the end
        if !s.ends_with(')') {
            return None;
        }

        let inner = &s[open_idx + 1..s.len() - 1];
        if inner.is_empty() {
            return None;
        }

        if !is_known_tool(tool_name) {
            return None;
        }

        let tool_pattern = ToolPattern {
            name: tool_name.to_lowercase(),
            pattern: inner.to_string(),
        };

        Some((Some(tool_pattern), inner.to_string()))
    } else {
        // No parentheses — bare pattern, applies to all tools
        Some((None, s.to_string()))
    }
}

/// Compile a glob pattern string into a `GlobMatcher`.
///
/// Returns an error message if the pattern is invalid.
pub fn compile_glob(pattern: &str) -> Result<GlobMatcher, String> {
    let glob = Glob::new(pattern).map_err(|e| format!("invalid glob pattern: {e}"))?;
    Ok(glob.compile_matcher())
}

/// Check whether a path matches a glob pattern.
///
/// The path is matched in three ways:
/// 1. As a relative path joined with `workspace_root`
/// 2. As an absolute path (used as-is)
/// 3. With `~/` resolved to the user's home directory
pub fn path_matches_glob(path: &Path, pattern: &str, workspace_root: &Path) -> bool {
    let matcher = match compile_glob(pattern) {
        Ok(m) => m,
        Err(_) => return false,
    };

    // 1. Try relative path joined with workspace_root
    if let Ok(relative) = path.strip_prefix(workspace_root) {
        if matcher.is_match(relative) {
            return true;
        }
    }

    // Also try the path as-is (could be relative to workspace_root without the prefix)
    if matcher.is_match(path) {
        return true;
    }

    // 2. Try absolute path
    if path.is_absolute() && matcher.is_match(path) {
        return true;
    }

    // 3. Try ~/ path resolution
    let path_str = path.to_string_lossy();
    if path_str.starts_with("~/") || path_str == "~" {
        if let Some(home) = home::home_dir() {
            if path_str == "~" {
                if matcher.is_match(&home) {
                    return true;
                }
            } else {
                let resolved = home.join(&path_str[2..]);
                if matcher.is_match(&resolved) {
                    return true;
                }
            }
        }
    }

    false
}

/// Build a session cache key from a tool name and pattern string.
///
/// Format: `"tool:pattern"` (tool is lowercased).
pub fn session_key(tool: &str, pattern: &str) -> String {
    format!("{}:{}", tool.to_lowercase(), pattern)
}

/// Check whether a command string matches a pattern (case-insensitive substring match).
pub fn command_matches_pattern(command: &str, pattern: &str) -> bool {
    let command_lower = command.to_lowercase();
    let pattern_lower = pattern.to_lowercase();
    command_lower.contains(&pattern_lower)
}

/// Resolve a pattern path to an absolute path string.
///
/// - `~/...` paths are resolved against the user's home directory
/// - Absolute paths (starting with `/`) are returned as-is
/// - Relative paths are joined with `workspace_root`
pub fn resolve_pattern_path(pattern: &str, workspace_root: &Path) -> String {
    if pattern.starts_with("~/") || pattern == "~" {
        if let Some(home) = home::home_dir() {
            if pattern == "~" {
                return home.to_string_lossy().to_string();
            }
            return home.join(&pattern[2..]).to_string_lossy().to_string();
        }
        // If home dir is not available, return the original pattern as-is
        // rather than producing a path like /workspace/~/config.json
        pattern.to_string()
    } else if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        workspace_root.join(pattern).to_string_lossy().to_string()
    }
}

/// Check whether a rule applies to a given tool.
///
/// Returns `true` if the rule has no tool pattern restriction,
/// or if the tool name matches the rule's tool pattern (case-insensitive).
pub fn rule_matches_tool(rule: &PermissionRule, tool: &str) -> bool {
    match &rule.tool_pattern {
        Some(tp) => tp.name.eq_ignore_ascii_case(tool),
        None => true,
    }
}

/// Check whether a rule applies to a given file operation.
///
/// In the current data model, `PermissionRule` does not carry an operation-level
/// filter (e.g. "only applies to Read, not Write"). As a result:
///
/// - **Path-only rules** (those with a `path_pattern` but no `command_pattern`)
///   are considered to apply to **all** file operations — both `Read` and `Write`.
///   This matches the semantics of config-level rules where a path pattern like
///   `".env"` should protect the file regardless of the operation being performed.
/// - **Command-only rules** (those with a `command_pattern` but no `path_pattern`)
///   do not apply to any file operation and return `false`.
/// - **Combined rules** (both `path_pattern` and `command_pattern`) apply to all
///   file operations, same as path-only rules.
///
/// The `op` parameter is accepted for future use when rules gain per-operation
/// filtering, but is not currently consulted.
pub fn rule_matches_operation(rule: &PermissionRule, _op: &FileOp) -> bool {
    // Rules with no path pattern are command-only rules, not operation rules
    if rule.path_pattern.is_none() {
        return false;
    }

    // If the rule has no command pattern, it's a path-only rule
    // Path-only rules apply to all operations
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionRule, RuleAction, ToolPattern};
    use std::path::Path;

    // ── is_known_tool ────────────────────────────────────────────────────

    #[test]
    fn test_is_known_tool_bash() {
        assert!(is_known_tool("bash"));
    }

    #[test]
    fn test_is_known_tool_read() {
        assert!(is_known_tool("Read"));
    }

    #[test]
    fn test_is_known_tool_write() {
        assert!(is_known_tool("WRITE"));
    }

    #[test]
    fn test_is_known_tool_edit() {
        assert!(is_known_tool("edit"));
    }

    #[test]
    fn test_is_known_tool_search_grep() {
        assert!(is_known_tool("search_grep"));
    }

    #[test]
    fn test_is_known_tool_searchgrep() {
        assert!(is_known_tool("searchgrep"));
    }

    #[test]
    fn test_is_known_tool_unknown() {
        assert!(!is_known_tool("unknown_tool"));
    }

    #[test]
    fn test_is_known_tool_case_insensitive() {
        assert!(is_known_tool("BASH"));
        assert!(is_known_tool("READ"));
        assert!(is_known_tool("Search_Grep"));
    }

    // ── parse_permission_string ──────────────────────────────────────────

    #[test]
    fn test_parse_read_permission() {
        let result = parse_permission_string("Read(./.env)");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_some());
        let tp = tool_pattern.unwrap();
        assert_eq!(tp.name, "read");
        assert_eq!(tp.pattern, "./.env");
        assert_eq!(inner, "./.env");
    }

    #[test]
    fn test_parse_bash_permission() {
        let result = parse_permission_string("Bash(echo hello)");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_some());
        let tp = tool_pattern.unwrap();
        assert_eq!(tp.name, "bash");
        assert_eq!(tp.pattern, "echo hello");
        assert_eq!(inner, "echo hello");
    }

    #[test]
    fn test_parse_write_permission() {
        let result = parse_permission_string("Write(/path/to/file)");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_some());
        let tp = tool_pattern.unwrap();
        assert_eq!(tp.name, "write");
        assert_eq!(tp.pattern, "/path/to/file");
        assert_eq!(inner, "/path/to/file");
    }

    #[test]
    fn test_parse_bare_pattern_no_tool() {
        let result = parse_permission_string("./.env");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_none());
        assert_eq!(inner, "./.env");
    }

    #[test]
    fn test_parse_bare_pattern_absolute() {
        let result = parse_permission_string("/etc/passwd");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_none());
        assert_eq!(inner, "/etc/passwd");
    }

    #[test]
    fn test_parse_empty_string_rejected() {
        let result = parse_permission_string("");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_no_parens_is_bare_not_error() {
        // "Read" without parens is not a ToolName(pattern) — it's a bare pattern
        let result = parse_permission_string("Read");
        assert!(result.is_some());
        let (tool_pattern, inner) = result.unwrap();
        assert!(tool_pattern.is_none());
        assert_eq!(inner, "Read");
    }

    #[test]
    fn test_parse_empty_tool_name_rejected() {
        let result = parse_permission_string("(./foo)");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_empty_pattern_rejected() {
        let result = parse_permission_string("Read()");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_unknown_tool_rejected() {
        let result = parse_permission_string("UnknownTool(./foo)");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_tool_name_case_insensitive() {
        let result = parse_permission_string("READ(./.env)");
        assert!(result.is_some());
        let (tool_pattern, _) = result.unwrap();
        assert!(tool_pattern.is_some());
        assert_eq!(tool_pattern.unwrap().name, "read");
    }

    // ── compile_glob ─────────────────────────────────────────────────────

    #[test]
    fn test_compile_valid_glob() {
        let matcher = compile_glob("**/*.env");
        assert!(matcher.is_ok());
    }

    #[test]
    fn test_compile_invalid_glob() {
        let result = compile_glob("[invalid");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid glob pattern"));
    }

    #[test]
    fn test_compile_glob_matches() {
        let matcher = compile_glob("*.rs").unwrap();
        assert!(matcher.is_match("lib.rs"));
        assert!(!matcher.is_match("lib.txt"));
    }

    // ── path_matches_glob ────────────────────────────────────────────────

    #[test]
    fn test_path_matches_glob_relative_pattern() {
        let matcher = compile_glob("**/*.env").unwrap();
        assert!(matcher.is_match(Path::new(".env")));
        assert!(matcher.is_match(Path::new("subdir/.env")));
    }

    #[test]
    fn test_path_matches_glob_in_workspace() {
        let workspace = Path::new("/workspace");
        // A path that is within the workspace
        assert!(path_matches_glob(
            &workspace.join("src/main.rs"),
            "src/main.rs",
            workspace,
        ));
    }

    #[test]
    fn test_path_matches_glob_absolute() {
        let workspace = Path::new("/workspace");
        // An absolute path outside workspace
        assert!(path_matches_glob(
            Path::new("/etc/passwd"),
            "/etc/passwd",
            workspace,
        ));
    }

    #[test]
    fn test_path_matches_glob_no_match() {
        let workspace = Path::new("/workspace");
        assert!(!path_matches_glob(
            &workspace.join("Cargo.toml"),
            "*.rs",
            workspace,
        ));
    }

    #[test]
    fn test_path_matches_glob_invalid_pattern() {
        let workspace = Path::new("/workspace");
        // Invalid pattern should return false, not panic
        assert!(!path_matches_glob(
            &workspace.join("foo.txt"),
            "[invalid",
            workspace,
        ));
    }

    // ── session_key ──────────────────────────────────────────────────────

    #[test]
    fn test_session_key_format() {
        assert_eq!(session_key("Read", "./.env"), "read:./.env");
    }

    #[test]
    fn test_session_key_lowercases_tool() {
        assert_eq!(session_key("BASH", "echo hi"), "bash:echo hi");
    }

    #[test]
    fn test_session_key_preserves_pattern_case() {
        assert_eq!(session_key("write", "./Foo.Bar"), "write:./Foo.Bar");
    }

    // ── command_matches_pattern ──────────────────────────────────────────

    #[test]
    fn test_command_matches_substring() {
        assert!(command_matches_pattern("echo hello world", "hello"));
    }

    #[test]
    fn test_command_matches_case_insensitive() {
        assert!(command_matches_pattern("ECHO hello", "echo"));
    }

    #[test]
    fn test_command_matches_case_insensitive_reverse() {
        assert!(command_matches_pattern("echo hello", "ECHO"));
    }

    #[test]
    fn test_command_matches_no_match() {
        assert!(!command_matches_pattern("ls -la", "rm"));
    }

    #[test]
    fn test_command_matches_empty_pattern() {
        assert!(command_matches_pattern("anything", ""));
    }

    #[test]
    fn test_command_matches_empty_command() {
        assert!(!command_matches_pattern("", "something"));
    }

    // ── resolve_pattern_path ─────────────────────────────────────────────

    #[test]
    fn test_resolve_relative_path() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_pattern_path("src/main.rs", workspace);
        assert_eq!(resolved, "/workspace/src/main.rs");
    }

    #[test]
    fn test_resolve_absolute_path() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_pattern_path("/etc/passwd", workspace);
        assert_eq!(resolved, "/etc/passwd");
    }

    #[test]
    fn test_resolve_tilde_path() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_pattern_path("~/config.json", workspace);
        // Should resolve ~ to home dir
        if let Some(home) = home::home_dir() {
            let expected = home.join("config.json").to_string_lossy().to_string();
            assert_eq!(resolved, expected);
        }
    }

    #[test]
    fn test_resolve_tilde_only() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_pattern_path("~", workspace);
        if let Some(home) = home::home_dir() {
            assert_eq!(resolved, home.to_string_lossy().to_string());
        }
    }

    // ── rule_matches_tool ────────────────────────────────────────────────

    #[test]
    fn test_rule_matches_tool_no_tool_pattern() {
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        };
        assert!(rule_matches_tool(&rule, "bash"));
        assert!(rule_matches_tool(&rule, "read"));
    }

    #[test]
    fn test_rule_matches_tool_matching() {
        let rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "./*".into(),
            }),
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        };
        assert!(rule_matches_tool(&rule, "read"));
    }

    #[test]
    fn test_rule_matches_tool_case_insensitive() {
        let rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "./*".into(),
            }),
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        };
        assert!(rule_matches_tool(&rule, "READ"));
    }

    #[test]
    fn test_rule_matches_tool_not_matching() {
        let rule = PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "read".into(),
                pattern: "./*".into(),
            }),
            path_pattern: None,
            command_pattern: None,
            action: RuleAction::Allow,
        };
        assert!(!rule_matches_tool(&rule, "bash"));
    }

    // ── rule_matches_operation ───────────────────────────────────────────

    #[test]
    fn test_rule_matches_operation_path_rule() {
        // A path-only rule (has path_pattern, no command_pattern)
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/*.env".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        };
        assert!(rule_matches_operation(&rule, &FileOp::Read));
        assert!(rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_command_only_rule() {
        // A command-only rule (has command_pattern, no path_pattern)
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("rm".into()),
            action: RuleAction::Deny,
        };
        // Command-only rules don't apply to file operations
        assert!(!rule_matches_operation(&rule, &FileOp::Read));
        assert!(!rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_path_rule_applies_to_read_and_write() {
        // Path-only rules apply to ALL operations (current semantics)
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/*.env".into()),
            command_pattern: None,
            action: RuleAction::Deny,
        };
        // Both Read and Write are matched
        assert!(rule_matches_operation(&rule, &FileOp::Read));
        assert!(rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_combined_rule_applies_to_all_ops() {
        // A combined rule (both path and command) also applies to all operations
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/secrets/*".into()),
            command_pattern: Some("cat".into()),
            action: RuleAction::Deny,
        };
        assert!(rule_matches_operation(&rule, &FileOp::Read));
        assert!(rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_op_param_accepted_but_not_consulted() {
        // Verify the current semantic: path-only rules ignore the specific op
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.txt".into()),
            command_pattern: None,
            action: RuleAction::Allow,
        };
        // All file ops get the same answer
        assert_eq!(
            rule_matches_operation(&rule, &FileOp::Read),
            rule_matches_operation(&rule, &FileOp::Write),
        );
    }
}
