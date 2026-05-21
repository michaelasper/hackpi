use super::matcher::{check_against_dangerous_patterns, check_command_against_rules};
use crate::{GuardResult, PermissionRule};

/// Check a command string against the command gate rules.
///
/// Evaluation order:
/// 1. Configured rules with `command_pattern` (from config)
/// 2. Built-in dangerous patterns (from `DANGEROUS_PATTERNS`)
/// 3. No matches → `Allow`
///
/// Returns `Allow` if no patterns match, `Deny` if a deny pattern matches,
/// or `Ask` if an ask pattern matches.
pub fn check(command: &str, rules: &[PermissionRule], tool: &str) -> GuardResult {
    // 1. Check configured rules with command_pattern first (overrides built-ins)
    if let Some(result) = check_command_against_rules(command, rules, tool) {
        return result;
    }

    // 2. Check built-in dangerous patterns as fallback
    if let Some(result) = check_against_dangerous_patterns(command, tool) {
        return result;
    }

    // 3. No matches → Allow
    GuardResult::Allow
}
