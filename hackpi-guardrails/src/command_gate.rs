use crate::{GuardResult, PermissionRule};

/// Check a command string against the command gate rules.
///
/// Scans the command for dangerous patterns. Returns `Allow` if no
/// patterns match, `Deny` if a deny pattern matches, or `Ask` if an
/// ask pattern matches.
pub fn check(_command: &str, _rules: &[PermissionRule]) -> GuardResult {
    // TODO: Implement command scanning (Phase 5)
    GuardResult::Allow
}
