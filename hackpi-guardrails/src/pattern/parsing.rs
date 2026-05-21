use crate::ToolPattern;
use globset::{Glob, GlobMatcher};

/// Known tool names recognized by the guard system.
const KNOWN_TOOLS: &[&str] = &[
    "bash",
    "read",
    "write",
    "edit",
    "search_grep",
    "searchgrep",
    "git_write",
];

/// Check if a tool name is one of the known tools.
///
/// Comparison is case-insensitive.
pub fn is_known_tool(name: &str) -> bool {
    KNOWN_TOOLS.iter().any(|t| t.eq_ignore_ascii_case(name))
}

/// Parse a permission string in the format `ToolName(pattern)` (Claude Code style).
///
/// The tool name portion is case-insensitive. If no tool prefix is detected
/// (i.e., the string does not contain parentheses), returns the entire string
/// as a bare pattern with no tool restriction.
///
/// Returns `None` when:
/// - The string is empty
/// - The format has parentheses but the tool name is empty
/// - The parentheses are present but the inner pattern is empty
/// - The tool name before the parentheses is not a known tool
pub fn parse_permission_string(s: &str) -> Option<(Option<ToolPattern>, String)> {
    if s.is_empty() {
        return None;
    }

    // Check for ToolName(pattern) format
    if let Some(open_idx) = s.find('(') {
        let tool_name = &s[..open_idx];
        if tool_name.is_empty() {
            return None;
        }

        // Must have closing paren at the end
        if !s.ends_with(')') {
            return None;
        }

        let inner = &s[open_idx + 1..s.len() - 1];
        if inner.is_empty() {
            return None;
        }

        if !is_known_tool(tool_name) {
            return None;
        }

        let tool_pattern = ToolPattern {
            name: tool_name.to_lowercase(),
            pattern: inner.to_string(),
        };

        Some((Some(tool_pattern), inner.to_string()))
    } else {
        // No parentheses — bare pattern, applies to all tools
        Some((None, s.to_string()))
    }
}

/// Compile a glob pattern string into a `GlobMatcher`.
///
/// Returns an error message if the pattern is invalid.
pub fn compile_glob(pattern: &str) -> Result<GlobMatcher, String> {
    let glob = Glob::new(pattern).map_err(|e| format!("invalid glob pattern: {e}"))?;
    Ok(glob.compile_matcher())
}

/// Build a session cache key from a tool name and pattern string.
///
/// Format: `"tool:pattern"` (tool is lowercased).
pub fn session_key(tool: &str, pattern: &str) -> String {
    format!("{}:{}", tool.to_lowercase(), pattern)
}
