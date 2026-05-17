use crate::{PermissionRule, SettingsPaths};

/// Load and merge permission rules from all configured sources.
///
/// Reads `.hackpi/guardrails.json`, `.claude/settings.json`, and
/// `.claude/settings.local.json`, parses them, and merges by priority.
///
/// Returns an empty Vec if no config files exist yet.
pub fn load_all(_paths: &SettingsPaths) -> Result<Vec<PermissionRule>, String> {
    // TODO: Implement config parsing (Phase 3)
    Ok(Vec::new())
}
