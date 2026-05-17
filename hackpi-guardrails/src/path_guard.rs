use crate::{GuardResult, PermissionRule};
use std::path::Path;

/// Check a path against path access rules.
///
/// Enforces workspace boundaries and applies allow/deny path patterns.
/// Returns `Allow` if the path is within bounds, `Deny` if blocked,
/// or `Ask` if user input is needed.
pub fn check(_path: &Path, _rules: &[PermissionRule]) -> GuardResult {
    // TODO: Implement path guard (Phase 4)
    GuardResult::Allow
}
