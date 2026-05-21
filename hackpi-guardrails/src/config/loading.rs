use crate::{PermissionRule, SettingsPaths};
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::parsing;

/// Load and merge permission rules from all configured sources.
///
/// Reads `.hackpi/guardrails.json`, `.claude/settings.local.json`, and
/// `.claude/settings.json`, parses them, and merges by priority.
///
/// Priority (highest first):
/// 1. `.hackpi/guardrails.json` — project-specific, committed
/// 2. `.claude/settings.local.json` — personal overrides, gitignored
/// 3. `.claude/settings.json` — team-wide defaults, checked in
///
/// Non-existent files are silently skipped.
/// Returns an empty Vec if no config files exist.
pub fn load_all(paths: &SettingsPaths) -> Result<Vec<PermissionRule>, String> {
    let mut all_rules = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    // 1. .hackpi/guardrails.json (highest priority)
    if paths.hackpi.exists() {
        match load_hackpi_config(&paths.hackpi) {
            Ok(rules) => all_rules.extend(rules),
            Err(e) => errors.push(format!("{}: {e}", paths.hackpi.display())),
        }
    }

    // 2. .claude/settings.local.json
    if paths.claude_local.exists() {
        match load_claude_settings(&paths.claude_local) {
            Ok(rules) => all_rules.extend(rules),
            Err(e) => errors.push(format!("{}: {e}", paths.claude_local.display())),
        }
    }

    // 3. .claude/settings.json (lowest priority)
    if paths.claude_project.exists() {
        match load_claude_settings(&paths.claude_project) {
            Ok(rules) => all_rules.extend(rules),
            Err(e) => errors.push(format!("{}: {e}", paths.claude_project.display())),
        }
    }

    // If ALL files failed, return the aggregated errors
    if all_rules.is_empty() && !errors.is_empty() {
        return Err(format!(
            "Failed to load all config files:\n  {}",
            errors.join("\n  ")
        ));
    }

    // If some files failed but others succeeded, log warnings
    if !errors.is_empty() {
        tracing::warn!(
            "Guardrails: some config files failed to load:\n  {}",
            errors.join("\n  ")
        );
    }

    Ok(all_rules)
}

/// Parse `.hackpi/guardrails.json` format.
///
/// Supports four sections:
/// - `permissions` — allow/deny arrays of `ToolName(pattern)` strings
/// - `path_access` — path-based allow/deny with optional catch-all ask
/// - `command_gate` — command pattern map with ask/deny actions
/// - `file_protection` — file pattern map with per-operation actions
pub fn load_hackpi_config(path: &Path) -> Result<Vec<PermissionRule>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let config: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

    let mut rules: Vec<PermissionRule> = Vec::new();

    // Parse permissions block
    if let Some(perms) = config.get("permissions") {
        let allow = perms.get("allow").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
        let deny = perms.get("deny").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

        let perm_rules = parsing::parse_permissions_block(allow.as_deref(), deny.as_deref())
            .map_err(|e| format!("permissions: {e}"))?;
        rules.extend(perm_rules);
    }

    // Parse path_access block
    if let Some(pa) = config.get("path_access") {
        let pa_rules =
            parsing::parse_path_access_block(pa).map_err(|e| format!("path_access: {e}"))?;
        rules.extend(pa_rules);
    }

    // Parse command_gate block
    if let Some(cg) = config.get("command_gate") {
        let cg_rules =
            parsing::parse_command_gate_block(cg).map_err(|e| format!("command_gate: {e}"))?;
        rules.extend(cg_rules);

        // Check allow_git_in_bash — inject bypass rules at the front
        let extras = parsing::parse_command_gate_extras(cg);
        if extras.allow_git_in_bash {
            let mut bypass = parsing::vcs_bypass_rules();
            bypass.extend(rules);
            rules = bypass;
        }
    }

    // Parse file_protection block
    if let Some(fp) = config.get("file_protection") {
        let fp_rules = parsing::parse_file_protection_block(fp)
            .map_err(|e| format!("file_protection: {e}"))?;
        rules.extend(fp_rules);
    }

    Ok(rules)
}

/// Parse `.claude/settings.json` or `.claude/settings.local.json`.
///
/// Extracts the `permissions` block with allow/deny arrays of
/// `ToolName(pattern)` strings.
pub fn load_claude_settings(path: &Path) -> Result<Vec<PermissionRule>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;

    let config: Value = serde_json::from_str(&content)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;

    let mut rules = Vec::new();

    if let Some(perms) = config.get("permissions") {
        let allow = perms.get("allow").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
        let deny = perms.get("deny").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

        let perm_rules = parsing::parse_permissions_block(allow.as_deref(), deny.as_deref())
            .map_err(|e| format!("permissions: {e}"))?;
        rules.extend(perm_rules);
    }

    Ok(rules)
}
