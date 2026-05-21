use super::rules::DANGEROUS_PATTERNS;
use crate::{GuardReason, GuardResult, GuardType, PermissionRule, RuleAction};

/// Check a command against configured permission rules.
///
/// Only considers rules that have a `command_pattern` (path-only rules are
/// skipped). Rules are evaluated in config-list order (first-match-wins).
///
/// Uses `crate::pattern::rule_matches_tool()` for tool scoping and
/// `crate::pattern::command_matches_wildcard()` for case-insensitive
/// wildcard matching (`*` matches any sequence of characters).
pub fn check_command_against_rules(
    command: &str,
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

        // Check command matching with wildcard support
        if !crate::pattern::command_matches_wildcard(command, command_pattern) {
            continue;
        }

        return match rule.action {
            RuleAction::Deny => Some(GuardResult::Deny(format!(
                "Command '{}' is denied by rule matching '{}'",
                command, command_pattern,
            ))),
            RuleAction::Allow => Some(GuardResult::Allow),
            RuleAction::Ask => Some(GuardResult::Ask(GuardReason {
                guard: GuardType::CommandGate,
                tool: tool.to_string(),
                details: format!(
                    "Command '{}' matches pattern '{}'",
                    command, command_pattern,
                ),
            })),
        };
    }

    None
}

/// Check a command against the built-in `DANGEROUS_PATTERNS`.
///
/// Uses `crate::pattern::command_matches_at_word_boundary()` for patterns
/// with `word_boundary: true`, `crate::pattern::command_matches_pattern()`
/// with case-sensitivity controlled by `case_sensitive` for other patterns.
/// Returns the first matching pattern's `GuardResult`, or `None` if no
/// pattern matches.
///
/// Patterns with a `tool_scope` only match when the given `tool` matches
/// the scope (case-insensitive). Patterns without a `tool_scope` match
/// all tools.
pub fn check_against_dangerous_patterns(command: &str, tool: &str) -> Option<GuardResult> {
    for dp in DANGEROUS_PATTERNS {
        // Check tool scoping
        if let Some(scope) = dp.tool_scope {
            if !scope.eq_ignore_ascii_case(tool) {
                continue;
            }
        }

        let matches = if dp.word_boundary {
            crate::pattern::command_matches_at_word_boundary(command, dp.pattern, dp.case_sensitive)
        } else if dp.case_sensitive {
            command.contains(dp.pattern)
        } else {
            crate::pattern::command_matches_pattern(command, dp.pattern)
        };

        if matches {
            return match dp.action {
                RuleAction::Deny => Some(GuardResult::Deny(format!(
                    "Command '{}' matches dangerous pattern '{}'",
                    command, dp.pattern,
                ))),
                RuleAction::Ask => Some(GuardResult::Ask(GuardReason {
                    guard: GuardType::CommandGate,
                    tool: String::new(),
                    details: format!(
                        "Command '{}' matches dangerous pattern '{}'",
                        command, dp.pattern,
                    ),
                })),
                RuleAction::Allow => continue,
            };
        }
    }
    None
}
