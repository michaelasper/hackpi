use crate::{GuardResult, PermissionRule};
use std::path::Path;

/// Check a path against file protection rules.
///
/// Protects sensitive files (`.env`, secrets, credentials, keys) from
/// accidental access. Returns `Allow` if no rules match, `Deny` if a
/// deny rule matches, or `Ask` if an ask rule matches.
pub fn check(_path: &Path, _rules: &[PermissionRule]) -> GuardResult {
    // TODO: Implement file protection (Phase 6)
    GuardResult::Allow
}
