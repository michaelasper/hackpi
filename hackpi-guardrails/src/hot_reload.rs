use crate::PermissionRule;

/// Validate a set of permission rules without applying them.
///
/// Checks that all glob patterns compile, tool names are known, and
/// command patterns are non-empty.
pub fn validate(_rules: &[PermissionRule]) -> Result<(), String> {
    // TODO: Implement full validation (Phase 7)
    Ok(())
}
