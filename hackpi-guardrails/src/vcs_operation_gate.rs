use crate::{GuardReason, GuardResult, GuardType, PermissionRule, RuleAction};

/// A built-in destructive VCS operation pattern with its action.
pub struct DestructiveVcsOp {
    /// The operation name (e.g., "reset", "push", "branch_delete").
    /// Matching is case-insensitive exact match on the operation field.
    pub operation: &'static str,
    /// Optional param conditions: (param_key, expected_value).
    /// When set, the operation matches only if the param exists with the given value.
    /// When None, the operation matches regardless of params.
    pub param_conditions: Option<&'static [(&'static str, &'static str)]>,
    /// The action to take when this operation matches.
    pub action: RuleAction,
}

/// Built-in destructive git_write operation patterns.
///
/// These are checked as a fallback after config rules. Destructive
/// operations are flagged to prevent the model from destroying
/// uncommitted work, rewriting repository state, deleting branches,
/// or pushing changes without guardrail oversight.
///
/// More specific patterns (with param conditions) must come before
/// less specific ones since the first match wins.
pub const DESTRUCTIVE_VCS_OPERATIONS: &[DestructiveVcsOp] = &[
    // ── Deny patterns (highest severity) ──────────────────────────────
    // Reset --hard destroys uncommitted work in working directory and index
    DestructiveVcsOp {
        operation: "reset",
        param_conditions: Some(&[("mode", "hard")]),
        action: RuleAction::Deny,
    },
    // Force push rewrites remote history
    DestructiveVcsOp {
        operation: "push",
        param_conditions: Some(&[("force", "true")]),
        action: RuleAction::Deny,
    },
    // ── Ask patterns (potentially destructive but may be legitimate) ──
    // Reset (soft/mixed) modifies HEAD/index
    DestructiveVcsOp {
        operation: "reset",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Branch deletion removes a branch entirely
    DestructiveVcsOp {
        operation: "branch_delete",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Push sends commits to remote (without force)
    DestructiveVcsOp {
        operation: "push",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Merge creates merge commits and can alter branch topology
    DestructiveVcsOp {
        operation: "merge",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Rebase rewrites commit history
    DestructiveVcsOp {
        operation: "rebase",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Stash pop removes and applies stashed changes; can lose them on conflict
    DestructiveVcsOp {
        operation: "stash_pop",
        param_conditions: None,
        action: RuleAction::Ask,
    },
    // Checkout (branch switch) can discard uncommitted work
    DestructiveVcsOp {
        operation: "checkout",
        param_conditions: None,
        action: RuleAction::Ask,
    },
];

/// Check a git_write operation against guardrail rules.
///
/// Evaluation order:
/// 1. Configured rules with `command_pattern` (matching the operation name)
/// 2. Built-in destructive operation patterns (`DESTRUCTIVE_VCS_OPERATIONS`)
/// 3. No matches → `Allow` (non-destructive operations like add, commit, fetch)
///
/// Returns `Allow` for non-destructive operations, `Deny` for destructive
/// operations that are always blocked, or `Ask` for operations that need
/// user confirmation.
pub fn check(
    operation: &str,
    params: &serde_json::Value,
    rules: &[PermissionRule],
    tool: &str,
) -> GuardResult {
    // 1. Check configured rules with command_pattern first (overrides built-ins)
    if let Some(result) = check_vcs_operation_against_rules(operation, rules, tool) {
        return result;
    }

    // 2. Check built-in destructive operation patterns as fallback
    if let Some(result) = check_against_destructive_operations(operation, params) {
        return result;
    }

    // 3. No matches → Allow (non-destructive operation)
    GuardResult::Allow
}

/// Check a VCS operation against configured permission rules.
///
/// Only considers rules that have a `command_pattern` (path-only rules are
/// skipped) and match the current tool (git_write). The command pattern is
/// matched case-insensitively against the operation name.
///
/// This reuses the same rule infrastructure as command_gate: users can write
/// `GitWrite(reset)` or `GitWrite(push)` in their permission config to allow
/// or deny specific git_write operations.
fn check_vcs_operation_against_rules(
    operation: &str,
    rules: &[PermissionRule],
    tool: &str,
) -> Option<GuardResult> {
    for rule in rules {
        let command_pattern = match &rule.command_pattern {
            Some(p) => p,
            None => continue,
        };

        // Check tool scoping
        if !crate::pattern::rule_matches_tool(rule, tool) {
            continue;
        }

        // Check command matching (case-insensitive substring)
        if !crate::pattern::command_matches_pattern(operation, command_pattern) {
            continue;
        }

        return match rule.action {
            RuleAction::Deny => Some(GuardResult::Deny(format!(
                "Git write operation '{}' is denied by rule matching '{}'",
                operation, command_pattern,
            ))),
            RuleAction::Allow => Some(GuardResult::Allow),
            RuleAction::Ask => Some(GuardResult::Ask(GuardReason {
                guard: GuardType::GitWriteOperation,
                tool: tool.to_string(),
                details: format!(
                    "Git write operation '{}' matches pattern '{}'",
                    operation, command_pattern,
                ),
            })),
        };
    }

    None
}

/// Check a VCS operation against the built-in `DESTRUCTIVE_VCS_OPERATIONS`.
///
/// Returns the first matching pattern's `GuardResult`, or `None` if no
/// pattern matches (i.e., the operation is non-destructive).
fn check_against_destructive_operations(
    operation: &str,
    params: &serde_json::Value,
) -> Option<GuardResult> {
    for dp in DESTRUCTIVE_VCS_OPERATIONS {
        // Match operation name (case-insensitive)
        if !dp.operation.eq_ignore_ascii_case(operation) {
            continue;
        }

        // Check param conditions if specified
        if let Some(conditions) = dp.param_conditions {
            let all_match = conditions.iter().all(|(key, expected)| {
                let param_value = params.get(key);
                match param_value {
                    // String match (case-insensitive)
                    Some(v) if v.is_string() => v
                        .as_str()
                        .map(|s| s.eq_ignore_ascii_case(expected))
                        .unwrap_or(false),
                    // Boolean match: convert "true"/"false" to bool
                    Some(v) if v.is_boolean() => {
                        let bool_val = v.as_bool().unwrap_or(false);
                        let expected_bool = expected.eq_ignore_ascii_case("true");
                        bool_val == expected_bool
                    }
                    _ => false,
                }
            });
            if !all_match {
                continue;
            }
        }

        return match dp.action {
            RuleAction::Deny => Some(GuardResult::Deny(format!(
                "Destructive git write operation '{}' is denied by security policy",
                describe_operation(operation, &dp.param_conditions),
            ))),
            RuleAction::Ask => Some(GuardResult::Ask(GuardReason {
                guard: GuardType::GitWriteOperation,
                tool: String::new(),
                details: format!(
                    "Git write operation '{}' requires confirmation",
                    describe_operation(operation, &dp.param_conditions),
                ),
            })),
            RuleAction::Allow => continue,
        };
    }

    None
}

/// Build a human-readable description of an operation, including any
/// relevant params like mode or force.
fn describe_operation(
    operation: &str,
    param_conditions: &Option<&'static [(&'static str, &'static str)]>,
) -> String {
    match param_conditions {
        Some(conditions) if !conditions.is_empty() => {
            let params_str: Vec<String> =
                conditions.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!("{operation} ({})", params_str.join(", "))
        }
        _ => operation.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PermissionRule, ToolPattern};
    use serde_json::json;

    // ── Non-destructive operations ──────────────────────────────────────

    #[test]
    fn test_add_is_allowed() {
        let result = check("add", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_commit_is_allowed() {
        let result = check("commit", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_fetch_is_allowed() {
        let result = check("fetch", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_branch_create_is_allowed() {
        let result = check("branch_create", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_stash_is_allowed() {
        let result = check("stash", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    // ── Destructive operations → Deny ───────────────────────────────────

    #[test]
    fn test_reset_hard_is_denied() {
        let params = json!({ "mode": "hard" });
        let result = check("reset", &params, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                assert!(
                    msg.contains("reset"),
                    "deny msg should mention reset: {msg}"
                );
                assert!(
                    msg.contains("hard") || msg.contains("destructive"),
                    "deny msg should mention safety: {msg}"
                );
            }
            other => panic!("expected Deny for reset --hard, got {other:?}"),
        }
    }

    #[test]
    fn test_force_push_is_denied() {
        let params = json!({ "force": true });
        let result = check("push", &params, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("push") || msg_lower.contains("destructive"),
                    "deny msg should mention push: {msg}"
                );
            }
            other => panic!("expected Deny for push --force, got {other:?}"),
        }
    }

    // ── Destructive operations → Ask ────────────────────────────────────

    #[test]
    fn test_reset_soft_asks() {
        let params = json!({ "mode": "soft" });
        let result = check("reset", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("reset"),
                    "ask msg should mention reset: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for reset --soft, got {other:?}"),
        }
    }

    #[test]
    fn test_reset_mixed_asks() {
        let params = json!({ "mode": "mixed" });
        let result = check("reset", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("reset"),
                    "ask msg should mention reset: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for reset --mixed, got {other:?}"),
        }
    }

    #[test]
    fn test_reset_default_mixed_asks() {
        // Reset without mode param defaults to mixed → Ask
        let params = json!({ "revision": "HEAD~1" });
        let result = check("reset", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("reset"),
                    "ask msg should mention reset: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for reset (default), got {other:?}"),
        }
    }

    #[test]
    fn test_branch_delete_asks() {
        let params = json!({ "branch": "old-feature" });
        let result = check("branch_delete", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert_eq!(reason.guard, GuardType::GitWriteOperation);
                assert!(
                    reason.details.contains("branch_delete"),
                    "ask msg should mention branch_delete: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for branch_delete, got {other:?}"),
        }
    }

    #[test]
    fn test_push_asks_without_force() {
        let params = json!({ "remote": "origin", "branch": "main" });
        let result = check("push", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("push"),
                    "ask msg should mention push: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for push (no force), got {other:?}"),
        }
    }

    #[test]
    fn test_merge_asks() {
        let params = json!({ "branch": "feature" });
        let result = check("merge", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("merge"),
                    "ask msg should mention merge: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for merge, got {other:?}"),
        }
    }

    #[test]
    fn test_rebase_asks() {
        let params = json!({ "onto": "main" });
        let result = check("rebase", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("rebase"),
                    "ask msg should mention rebase: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for rebase, got {other:?}"),
        }
    }

    #[test]
    fn test_stash_pop_asks() {
        let params = json!({ "index": 0 });
        let result = check("stash_pop", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("stash_pop"),
                    "ask msg should mention stash_pop: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for stash_pop, got {other:?}"),
        }
    }

    #[test]
    fn test_checkout_asks() {
        let params = json!({ "branch": "main" });
        let result = check("checkout", &params, &[], "git_write");
        match result {
            GuardResult::Ask(reason) => {
                assert!(
                    reason.details.contains("checkout"),
                    "ask msg should mention checkout: {}",
                    reason.details
                );
            }
            other => panic!("expected Ask for checkout, got {other:?}"),
        }
    }

    // ── Config rules override built-ins ─────────────────────────────────

    #[test]
    fn test_allow_rule_overrides_reset_hard() {
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "git_write".into(),
                pattern: "*".into(),
            }),
            path_pattern: None,
            command_pattern: Some("reset".into()),
            operation: None,
            action: RuleAction::Allow,
        }];
        let params = json!({ "mode": "hard", "revision": "HEAD~1" });
        let result = check("reset", &params, &rules, "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_allow_rule_overrides_force_push() {
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "git_write".into(),
                pattern: "*".into(),
            }),
            path_pattern: None,
            command_pattern: Some("push".into()),
            operation: None,
            action: RuleAction::Allow,
        }];
        let params = json!({ "force": true, "remote": "origin" });
        let result = check("push", &params, &rules, "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    #[test]
    fn test_deny_rule_overrides_default_allow_for_add() {
        let rules = vec![PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("add".into()),
            operation: None,
            action: RuleAction::Deny,
        }];
        let result = check("add", &json!({}), &rules, "git_write");
        match result {
            GuardResult::Deny(msg) => {
                assert!(msg.contains("add"), "deny msg should mention add: {msg}");
            }
            other => panic!("expected Deny for add with deny rule, got {other:?}"),
        }
    }

    // ── Tool-scoped rules ───────────────────────────────────────────────

    #[test]
    fn test_allow_rule_scoped_to_different_tool_does_not_apply() {
        // Rule is scoped to "bash", not "git_write"
        let rules = vec![PermissionRule {
            tool_pattern: Some(ToolPattern {
                name: "bash".into(),
                pattern: "*".into(),
            }),
            path_pattern: None,
            command_pattern: Some("reset".into()),
            operation: None,
            action: RuleAction::Allow,
        }];
        // Built-in should still flag reset
        let params = json!({ "mode": "hard" });
        let result = check("reset", &params, &rules, "git_write");
        match result {
            GuardResult::Deny(msg) => {
                assert!(
                    msg.contains("destructive") || msg.contains("reset"),
                    "deny msg should mention reset: {msg}"
                );
            }
            other => panic!("expected Deny from built-in, got {other:?}"),
        }
    }

    // ── Unknown operations ──────────────────────────────────────────────

    #[test]
    fn test_unknown_operation_is_allowed() {
        // Unknown operations fall through to Allow (they'll fail at execution time)
        let result = check("unknown_op", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    // ── Edge case: case insensitivity ───────────────────────────────────

    #[test]
    fn test_operation_case_insensitive() {
        let params_reset = json!({ "mode": "hard" });
        let result = check("RESET", &params_reset, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("reset") || msg_lower.contains("destructive"),
                    "deny msg for RESET: {msg}"
                );
            }
            other => panic!("expected Deny for RESET, got {other:?}"),
        }

        let params_push = json!({ "force": true });
        let result = check("PUSH", &params_push, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("push") || msg_lower.contains("destructive"),
                    "deny msg for PUSH: {msg}"
                );
            }
            other => panic!("expected Deny for PUSH, got {other:?}"),
        }
    }

    #[test]
    fn test_force_param_case_insensitive() {
        // Boolean true should match force param
        let params = json!({ "force": true });
        let result = check("push", &params, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("push") || msg_lower.contains("destructive"),
                    "deny msg for push --force: {msg}"
                );
            }
            other => panic!("expected Deny for push --force, got {other:?}"),
        }

        // Mode string case-insensitive
        let params = json!({ "mode": "HARD" });
        let result = check("reset", &params, &[], "git_write");
        match result {
            GuardResult::Deny(msg) => {
                let msg_lower = msg.to_lowercase();
                assert!(
                    msg_lower.contains("reset") || msg_lower.contains("destructive"),
                    "deny msg for RESET HARD: {msg}"
                );
            }
            other => panic!("expected Deny for reset HARD, got {other:?}"),
        }
    }

    // ── Pull test (non-destructive) ─────────────────────────────────────

    #[test]
    fn test_pull_is_allowed() {
        // Pull fetches and merges, which is generally safe
        let result = check("pull", &json!({}), &[], "git_write");
        assert_eq!(result, GuardResult::Allow);
    }

    // ── DESTRUCTIVE_VCS_OPERATIONS const tests ──────────────────────────

    #[test]
    fn test_destructive_patterns_contain_expected_operations() {
        let ops: Vec<&str> = DESTRUCTIVE_VCS_OPERATIONS
            .iter()
            .map(|d| d.operation)
            .collect();
        assert!(ops.contains(&"reset"));
        assert!(ops.contains(&"push"));
        assert!(ops.contains(&"branch_delete"));
        assert!(ops.contains(&"merge"));
        assert!(ops.contains(&"rebase"));
        assert!(ops.contains(&"stash_pop"));
        assert!(ops.contains(&"checkout"));
    }

    #[test]
    fn test_reset_hard_before_reset() {
        // reset (hard) must come before reset (generic) so --hard matches first
        let idx_hard = DESTRUCTIVE_VCS_OPERATIONS
            .iter()
            .position(|d| d.operation == "reset" && d.param_conditions.is_some());
        let idx_generic = DESTRUCTIVE_VCS_OPERATIONS
            .iter()
            .position(|d| d.operation == "reset" && d.param_conditions.is_none());
        assert!(
            idx_hard < idx_generic,
            "reset (hard) must come before reset (generic) in DESTRUCTIVE_VCS_OPERATIONS"
        );
    }

    #[test]
    fn test_force_push_before_push() {
        // push (force) must come before push (generic) so --force matches first
        let idx_force = DESTRUCTIVE_VCS_OPERATIONS
            .iter()
            .position(|d| d.operation == "push" && d.param_conditions.is_some());
        let idx_generic = DESTRUCTIVE_VCS_OPERATIONS
            .iter()
            .position(|d| d.operation == "push" && d.param_conditions.is_none());
        assert!(
            idx_force < idx_generic,
            "push (force) must come before push (generic) in DESTRUCTIVE_VCS_OPERATIONS"
        );
    }
}
