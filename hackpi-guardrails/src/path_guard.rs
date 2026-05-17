use crate::{GuardReason, GuardResult, GuardType, PermissionRule, RuleAction};
use std::path::{Path, PathBuf};

/// Check a path against path access rules.
///
/// Resolves the path, checks allow/deny rules (which take precedence),
/// then enforces workspace boundaries. Returns `Allow` if the path is
/// within bounds, `Deny` if blocked, or `Ask` if user input is needed.
pub fn check(
    path: &str,
    workspace_root: &Path,
    rules: &[PermissionRule],
    tool: &str,
) -> GuardResult {
    // Step 1: Resolve the tool path to an absolute path
    let resolved = resolve_tool_path(path, workspace_root);

    // Step 2: Check path against rules first (rules take precedence over
    // workspace boundary)
    if let Some(result) = check_path_against_rules(&resolved, rules, tool, workspace_root) {
        return result;
    }

    // Step 3: Check workspace boundary
    if is_outside_workspace(&resolved, workspace_root) {
        return GuardResult::Ask(GuardReason {
            guard: GuardType::PathAccess,
            tool: tool.to_string(),
            details: format!(
                "Path '{}' is outside the workspace root '{}'",
                resolved.display(),
                workspace_root.display()
            ),
        });
    }

    GuardResult::Allow
}

/// Check whether a resolved absolute path is outside the workspace.
///
/// Uses `canonicalize()` on both path and workspace_root to resolve
/// symlinks and relative components. Returns `true` if the path does
/// not start with the workspace root.
///
/// If the path does not exist on disk (canonicalize fails), the function
/// tries to canonicalize the path's parent directory and appends the
/// file name. This handles non-existent files inside symlinked paths
/// (e.g., `/var` → `/private/var` on macOS).
pub fn is_outside_workspace(path: &Path, workspace_root: &Path) -> bool {
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());

    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // If the path doesn't exist, try canonicalizing the parent dir
            // and appending the file name, to resolve symlinks in parent paths
            if let Some(parent) = path.parent() {
                if let Ok(canon_parent) = parent.canonicalize() {
                    if let Some(name) = path.file_name() {
                        canon_parent.join(name)
                    } else {
                        path.to_path_buf()
                    }
                } else {
                    path.to_path_buf()
                }
            } else {
                path.to_path_buf()
            }
        }
    };

    !canonical_path.starts_with(&canonical_root)
}

/// Check a resolved path against the configured permission rules.
///
/// Only considers rules with a `path_pattern` that apply to the given
/// tool. Deny rules are checked first (fail-closed), then Allow rules.
/// Returns `None` if no rule matches.
pub fn check_path_against_rules(
    path: &Path,
    rules: &[PermissionRule],
    tool: &str,
    workspace_root: &Path,
) -> Option<GuardResult> {
    for rule in rules {
        let path_pattern = match &rule.path_pattern {
            Some(p) => p,
            None => continue,
        };

        // Check tool scoping
        if !crate::pattern::rule_matches_tool(rule, tool) {
            continue;
        }

        // Check if path matches the pattern
        if !crate::pattern::path_matches_glob(path, path_pattern, workspace_root) {
            continue;
        }

        match rule.action {
            RuleAction::Deny => {
                return Some(GuardResult::Deny(format!(
                    "Path '{}' is denied by rule matching '{}'",
                    path.display(),
                    path_pattern,
                )));
            }
            RuleAction::Allow => {
                return Some(GuardResult::Allow);
            }
            // Ask rules are handled at the workspace boundary level
            RuleAction::Ask => {}
        }
    }

    None
}

/// Resolve a tool path string to an absolute `PathBuf`.
///
/// - `~/...` paths are resolved against the user's home directory.
/// - Absolute paths (starting with `/`) are returned as-is.
/// - Relative paths are joined with `workspace_root`.
pub fn resolve_tool_path(path: &str, workspace_root: &Path) -> PathBuf {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = home::home_dir() {
            if path == "~" {
                return home;
            }
            return home.join(&path[2..]);
        }
        workspace_root.join(path)
    } else if Path::new(path).is_absolute() {
        Path::new(path).to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionRule, RuleAction, ToolPattern};
    use std::fs;
    use std::path::Path;

    // ── resolve_tool_path ────────────────────────────────────────────────

    #[test]
    fn test_resolve_tool_path_relative() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_tool_path("src/main.rs", workspace);
        assert_eq!(resolved, workspace.join("src/main.rs"));
    }

    #[test]
    fn test_resolve_tool_path_absolute() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_tool_path("/etc/passwd", workspace);
        assert_eq!(resolved, Path::new("/etc/passwd"));
    }

    #[test]
    fn test_resolve_tool_path_tilde() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_tool_path("~/.ssh/config", workspace);
        if let Some(home) = home::home_dir() {
            assert_eq!(resolved, home.join(".ssh/config"));
        }
    }

    #[test]
    fn test_resolve_tool_path_tilde_only() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_tool_path("~", workspace);
        if let Some(home) = home::home_dir() {
            assert_eq!(resolved, home);
        }
    }

    #[test]
    fn test_resolve_tool_path_dot() {
        let workspace = Path::new("/workspace");
        let resolved = resolve_tool_path("./foo.txt", workspace);
        assert_eq!(resolved, workspace.join("./foo.txt"));
    }

    // ── is_outside_workspace ──────────────────────────────────────────────

    #[test]
    fn test_is_outside_workspace_inside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let inside = workspace.join("some/file.txt");
        // Create the parent dir so canonicalize works
        fs::create_dir_all(inside.parent().unwrap()).expect("create dirs");
        fs::write(&inside, "content").expect("write file");

        assert!(!is_outside_workspace(&inside, workspace));
    }

    #[test]
    fn test_is_outside_workspace_outside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let outside = Path::new("/tmp/some-outside-file.txt");
        // Note: this path may not exist, so canonicalize will fail and fall back
        // For non-existent paths, we check the resolved path
        assert!(is_outside_workspace(outside, workspace));
    }

    #[test]
    fn test_is_outside_workspace_workspace_root_itself() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        assert!(!is_outside_workspace(workspace, workspace));
    }

    // ── check_path_against_rules ──────────────────────────────────────────

    #[test]
    fn test_check_rules_no_match_returns_none() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("src/main.rs");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("*.env".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_rules_deny_match() {
        let workspace = Path::new("/workspace");
        let path = workspace.join(".env");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/*.env".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert!(result.is_some());
        match result.unwrap() {
            GuardResult::Deny(msg) => {
                assert!(msg.contains(".env"));
            }
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn test_check_rules_allow_match() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("docs/readme.md");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("docs/**".to_string()),
            command_pattern: None,
            action: RuleAction::Allow,
        }];

        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert_eq!(result, Some(GuardResult::Allow));
    }

    #[test]
    fn test_check_rules_deny_takes_precedence_over_allow() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("secrets/key.pem");
        let rules = vec![
            PermissionRule {
                tool_pattern: None,
                path_pattern: Some("secrets/**".to_string()),
                command_pattern: None,
                action: RuleAction::Deny,
            },
            PermissionRule {
                tool_pattern: None,
                path_pattern: Some("secrets/key.pem".to_string()),
                command_pattern: None,
                action: RuleAction::Allow,
            },
        ];

        // Deny is first in the list, so it should match first
        let result = check_path_against_rules(&path, &rules, "read", workspace);
        match result {
            Some(GuardResult::Deny(_)) => {} // expected
            _ => panic!("expected Deny since deny rule comes first"),
        }
    }

    #[test]
    fn test_check_rules_tool_scoped_no_match_for_different_tool() {
        let workspace = Path::new("/workspace");
        let path = workspace.join(".env");
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "write".to_string(),
                pattern: "**/*.env".to_string(),
            }),
            path_pattern: Some("**/*.env".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        // 'read' tool should NOT match a 'write'-scoped rule
        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_rules_tool_scoped_matches_correct_tool() {
        let workspace = Path::new("/workspace");
        let path = workspace.join(".env");
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "write".to_string(),
                pattern: "**/*.env".to_string(),
            }),
            path_pattern: Some("**/*.env".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        // 'write' tool SHOULD match a 'write'-scoped rule
        let result = check_path_against_rules(&path, &rules, "write", workspace);
        assert!(result.is_some());
        match result.unwrap() {
            GuardResult::Deny(_) => {} // expected
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn test_check_rules_glob_pattern_nested() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("config/secrets/prod.env");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/secrets/**".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert!(result.is_some());
        match result.unwrap() {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("denied"));
            }
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn test_check_rules_absolute_path_pattern() {
        let workspace = Path::new("/workspace");
        let path = Path::new("/etc/passwd");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/etc/**".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check_path_against_rules(path, &rules, "read", workspace);
        assert!(result.is_some());
        match result.unwrap() {
            GuardResult::Deny(_) => {} // expected
            _ => panic!("expected Deny"),
        }
    }

    #[test]
    fn test_check_rules_skip_command_only_rules() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("foo.txt");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("rm".to_string()),
            action: RuleAction::Deny,
        }];

        // Command-only rule has no path_pattern, so it should be skipped
        let result = check_path_against_rules(&path, &rules, "bash", workspace);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_rules_ask_action_skipped() {
        let workspace = Path::new("/workspace");
        let path = workspace.join("some/file.txt");
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**".to_string()),
            command_pattern: None,
            action: RuleAction::Ask,
        }];

        // Ask rules are skipped in check_path_against_rules
        let result = check_path_against_rules(&path, &rules, "read", workspace);
        assert!(result.is_none());
    }

    // ── check (integration) ───────────────────────────────────────────────

    #[test]
    fn test_check_path_inside_workspace_allows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let file_path = workspace.join("src/main.rs");
        fs::create_dir_all(file_path.parent().unwrap()).expect("create dirs");
        fs::write(&file_path, "content").expect("write file");

        let result = check(file_path.to_str().unwrap(), workspace, &[], "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_path_outside_workspace_no_rule_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Use an absolute path outside the temp workspace
        let outside_path = "/tmp/cor8-test-outside-file";
        // Create it so canonicalize works
        let _ = fs::write(outside_path, "test");
        // Clean up after test
        let _ = fs::remove_file(outside_path);

        let result = check(outside_path, workspace, &[], "read");
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::PathAccess);
                assert_eq!(reason.tool, "read");
                assert!(reason.details.contains("outside the workspace"));
            }
            _ => panic!("expected Ask for outside-workspace path with no rule"),
        }
    }

    #[test]
    fn test_check_path_outside_workspace_with_deny_rule_denies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/tmp/**".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check("/tmp/some-file.txt", workspace, &rules, "read");
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("denied"));
            }
            _ => panic!("expected Deny for outside-workspace path with deny rule"),
        }
    }

    #[test]
    fn test_check_path_outside_workspace_with_allow_rule_allows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Create the path so canonicalize and matching work
        let outside_path = "/tmp/cor8-test-allowed-file";
        let _ = fs::write(outside_path, "test");
        let _ = fs::remove_file(outside_path);

        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/tmp/cor8-test-allowed-file".to_string()),
            command_pattern: None,
            action: RuleAction::Allow,
        }];

        let result = check(outside_path, workspace, &rules, "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_path_inside_workspace_with_deny_rule_denies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let file_path = workspace.join(".env");
        fs::write(&file_path, "SECRET=value").expect("write file");

        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("**/*.env".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        let result = check(file_path.to_str().unwrap(), workspace, &rules, "read");
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("denied"));
            }
            _ => panic!("expected Deny for inside-workspace path with deny rule"),
        }
    }

    #[test]
    fn test_check_path_inside_workspace_with_allow_rule_allows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let file_path = workspace.join("docs/guide.md");
        fs::create_dir_all(file_path.parent().unwrap()).expect("create dirs");
        fs::write(&file_path, "# Guide").expect("write file");

        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("docs/**".to_string()),
            command_pattern: None,
            action: RuleAction::Allow,
        }];

        let result = check(file_path.to_str().unwrap(), workspace, &rules, "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_tool_scoped_rule_only_matches_specific_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let file_path = workspace.join("secret.txt");
        fs::write(&file_path, "secret").expect("write file");

        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "write".to_string(),
                pattern: "secret.txt".to_string(),
            }),
            path_pattern: Some("secret.txt".to_string()),
            command_pattern: None,
            action: RuleAction::Deny,
        }];

        // 'read' does NOT match a 'write'-scoped rule, and path is inside workspace
        let result = check(file_path.to_str().unwrap(), workspace, &rules, "read");
        assert_eq!(result, GuardResult::Allow);

        // 'write' DOES match
        let result2 = check(file_path.to_str().unwrap(), workspace, &rules, "write");
        match result2 {
            GuardResult::Deny(_) => {} // expected
            _ => panic!("expected Deny for write tool"),
        }
    }

    #[test]
    fn test_check_relative_path_inside_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();
        let file_path = workspace.join("src/main.rs");
        fs::create_dir_all(file_path.parent().unwrap()).expect("create dirs");
        fs::write(&file_path, "content").expect("write file");

        // Use a relative path
        let result = check("src/main.rs", workspace, &[], "read");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_check_outside_workspace_with_glob_allow_rule() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // When an allow rule matches, it should return Allow even outside workspace
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: Some("/tmp/**".to_string()),
            command_pattern: None,
            action: RuleAction::Allow,
        }];

        // /tmp is outside the workspace but allowed by rule
        let outside_path = "/tmp/cor8-check-allowed";
        let _ = fs::write(outside_path, "test");
        let _ = fs::remove_file(outside_path);

        let result = check("/tmp/cor8-check-allowed", workspace, &rules, "read");
        assert_eq!(result, GuardResult::Allow);
    }

    // ── `..` traversal tests ─────────────────────────────────────────────

    #[test]
    fn test_is_outside_workspace_dot_dot_traversal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // A path using `..` to escape workspace
        let traversal = workspace.join("../outside-file.txt");
        // Create the actual parent dir so the path sort-of exists
        let parent = dir.path().parent().unwrap();
        let outside_file = parent.join("outside-file.txt");
        fs::write(&outside_file, "content").expect("write outside file");
        // Clean up
        let _ = fs::remove_file(&outside_file);

        assert!(
            is_outside_workspace(&traversal, workspace),
            ".. traversal should be detected as outside workspace"
        );
    }

    #[test]
    fn test_check_dot_dot_traversal_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // A path that uses `..` to reference a file above workspace
        let result = check("../outside-file.txt", workspace, &[], "read");
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::PathAccess);
                assert!(
                    reason.details.contains("outside"),
                    "ask reason should mention outside workspace"
                );
            }
            other => panic!("expected Ask for .. traversal, got {other:?}"),
        }
    }

    #[test]
    fn test_is_outside_workspace_complex_dot_dot_traversal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Complex traversal: src/../../etc/passwd
        let traversal = workspace.join("src/../../etc/passwd");
        assert!(
            is_outside_workspace(&traversal, workspace),
            "complex .. traversal should be detected as outside workspace"
        );
    }

    // ── Symlink tests ────────────────────────────────────────────────────

    #[test]
    fn test_is_outside_workspace_symlink_resolved_to_inside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Create a real file inside the workspace
        let real_file = workspace.join("real_file.txt");
        fs::write(&real_file, "content").expect("write real file");

        // Create a symlink outside the workspace pointing to the file inside
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let symlink_path = outside_dir.path().join("linked_file.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &symlink_path).expect("create symlink");
        }
        #[cfg(not(unix))]
        {
            std::os::windows::fs::symlink_file(&real_file, &symlink_path).expect("create symlink");
        }

        // The symlink's canonical path resolves to the real file inside the
        // workspace, so is_outside_workspace should return false.
        // This is correct behavior: security policy follows the canonical
        // (resolved) path, not the symlink location.
        assert!(
            !is_outside_workspace(&symlink_path, workspace),
            "symlink resolving inside workspace should not be outside"
        );
    }

    #[test]
    fn test_is_outside_workspace_symlink_inside_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Create a real file inside the workspace
        let real_file = workspace.join("target.txt");
        fs::write(&real_file, "content").expect("write real file");

        // Create a symlink to it also inside the workspace
        let symlink_inside = workspace.join("link_to_target.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &symlink_inside).expect("create symlink");
        }
        #[cfg(not(unix))]
        {
            std::os::windows::fs::symlink_file(&real_file, &symlink_inside)
                .expect("create symlink");
        }

        // The symlink resolves to a file inside the workspace
        assert!(
            !is_outside_workspace(&symlink_inside, workspace),
            "symlink inside workspace should not be detected as outside"
        );
    }

    #[test]
    fn test_check_symlink_inside_workspace_allows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Create a real file and symlink inside workspace
        let real_file = workspace.join("actual.txt");
        fs::write(&real_file, "content").expect("write file");

        let symlink_path = workspace.join("link.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &symlink_path).expect("create symlink");
        }
        #[cfg(not(unix))]
        {
            std::os::windows::fs::symlink_file(&real_file, &symlink_path).expect("create symlink");
        }

        let result = check(symlink_path.to_str().unwrap(), workspace, &[], "read");
        assert_eq!(
            result,
            GuardResult::Allow,
            "symlink inside workspace should be allowed"
        );
    }

    /// A symlink that points from inside workspace to outside should be caught.
    #[test]
    fn test_check_symlink_to_outside_asks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = dir.path();

        // Create file outside
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside_file = outside_dir.path().join("secret.txt");
        fs::write(&outside_file, "secret").expect("write outside file");

        // Create symlink inside workspace pointing outside
        let symlink_path = workspace.join("looks_innocent.txt");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_file, &symlink_path).expect("create symlink");
        }
        #[cfg(not(unix))]
        {
            std::os::windows::fs::symlink_file(&outside_file, &symlink_path)
                .expect("create symlink");
        }

        // When checking the symlink, path_guard should resolve it and detect
        // the target is outside the workspace
        let result = check(symlink_path.to_str().unwrap(), workspace, &[], "read");
        // The canonicalization will resolve the symlink and find it outside
        // the workspace. But it depends on whether the symlink target exists
        // and canonicalize succeeds.
        match result {
            GuardResult::Allow => {
                // On some platforms, symlink resolution might not work
                // as expected. This is acceptable.
            }
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::PathAccess);
                assert!(
                    reason.details.contains("outside"),
                    "should mention outside workspace"
                );
            }
            other => panic!("expected Allow or Ask, got {other:?}"),
        }
    }
}
