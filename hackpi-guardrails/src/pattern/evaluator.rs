use crate::{FileOp, PermissionRule};
use std::path::Path;

/// Resolve a pattern path to an absolute path string.
///
/// - `~/...` paths are resolved against the user's home directory
/// - Absolute paths (starting with `/`) are returned as-is
/// - Relative paths are joined with `workspace_root`
pub fn resolve_pattern_path(pattern: &str, workspace_root: &Path) -> String {
    if pattern.starts_with("~/") || pattern == "~" {
        if let Some(home) = home::home_dir() {
            if pattern == "~" {
                return home.to_string_lossy().to_string();
            }
            return home.join(&pattern[2..]).to_string_lossy().to_string();
        }
        // If home dir is not available, return the original pattern as-is
        // rather than producing a path like /workspace/~/config.json
        pattern.to_string()
    } else if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        workspace_root.join(pattern).to_string_lossy().to_string()
    }
}

/// Check whether a rule applies to a given tool.
///
/// Returns `true` if the rule has no tool pattern restriction,
/// or if the tool name matches the rule's tool pattern (case-insensitive).
pub fn rule_matches_tool(rule: &PermissionRule, tool: &str) -> bool {
    match &rule.tool_pattern {
        Some(tp) => tp.name.eq_ignore_ascii_case(tool),
        None => true,
    }
}

/// Check whether a rule applies to a given file operation.
///
/// - **Command-only rules** (those with a `command_pattern` but no `path_pattern`)
///   do not apply to any file operation and return `false`.
/// - **Path-only rules** with no explicit operation filter apply to **all**
///   file operations — both `Read` and `Write`.
/// - **Path-only rules** with an explicit `operation` filter only match
///   when the operation matches.
/// - **Combined rules** (both `path_pattern` and `command_pattern`) follow
///   the same operation-filtering logic.
pub fn rule_matches_operation(rule: &PermissionRule, op: &FileOp) -> bool {
    // Rules with no path pattern are command-only rules, not operation rules
    if rule.path_pattern.is_none() {
        return false;
    }

    // If the rule has an explicit operation filter, check it
    if let Some(rule_op) = &rule.operation {
        return rule_op == op;
    }

    // No operation filter — applies to all operations
    true
}
