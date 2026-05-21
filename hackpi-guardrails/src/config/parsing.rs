use serde_json::Value;

use crate::{PermissionRule, RuleAction};

use super::structs::CommandGateExtras;

/// Parse Claude Code-style permission strings.
///
/// Each string has the format `ToolName(pattern)` (e.g. `Read(./.env)`).
/// Malformed entries (unknown tools, empty strings, format errors) return
/// an error with a description of which entry was problematic. This ensures
/// that typos in deny/allow rules are never silently dropped.
///
/// Returns rules ordered with deny first, then allow (so that within a
/// single source, deny rules take precedence during first-match evaluation).
pub fn parse_permissions_block(
    allow: Option<&[String]>,
    deny: Option<&[String]>,
) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    // Deny rules first (deny beats allow within a source)
    if let Some(deny_strs) = deny {
        for (i, s) in deny_strs.iter().enumerate() {
            match permission_string_to_rule(s, RuleAction::Deny) {
                Ok(rule) => rules.push(rule),
                Err(e) => {
                    return Err(format!("deny[{i}]: {e}"));
                }
            }
        }
    }

    // Allow rules second
    if let Some(allow_strs) = allow {
        for (i, s) in allow_strs.iter().enumerate() {
            match permission_string_to_rule(s, RuleAction::Allow) {
                Ok(rule) => rules.push(rule),
                Err(e) => {
                    return Err(format!("allow[{i}]: {e}"));
                }
            }
        }
    }

    Ok(rules)
}

/// Convert a single permission string and action into a PermissionRule.
///
/// Returns an error if the string is malformed (invalid format, unknown tool, etc.).
fn permission_string_to_rule(s: &str, action: RuleAction) -> Result<PermissionRule, String> {
    let parsed =
        crate::pattern::parse_permission_string(s).ok_or_else(|| describe_permission_error(s))?;

    let (tool_pattern_str, pattern) = parsed;

    let tool_pattern = tool_pattern_str.map(|tp| crate::ToolPattern {
        name: tp.name,
        pattern: tp.pattern,
    });

    // Determine if this is a path or command pattern based on the tool
    let is_path_tool = tool_pattern
        .as_ref()
        .map(|tp| {
            matches!(
                tp.name.as_str(),
                "read" | "write" | "edit" | "search_grep" | "searchgrep"
            )
        })
        .unwrap_or(true);

    Ok(PermissionRule {
        tool_pattern,
        path_pattern: if is_path_tool {
            Some(pattern.clone())
        } else {
            None
        },
        command_pattern: if !is_path_tool { Some(pattern) } else { None },
        operation: None,
        action,
    })
}

/// Build a human-readable description of why a permission string failed to parse.
fn describe_permission_error(s: &str) -> String {
    if s.is_empty() {
        return "empty permission string".to_string();
    }
    if let Some(open_idx) = s.find('(') {
        let tool_name = &s[..open_idx];
        if tool_name.is_empty() {
            return format!("empty tool name in '{s}'");
        }
        if !s.ends_with(')') {
            return format!("missing closing parenthesis in '{s}'");
        }
        let inner = &s[open_idx + 1..s.len() - 1];
        if inner.is_empty() {
            return format!("empty pattern in '{s}'");
        }
        if !crate::pattern::is_known_tool(tool_name) {
            return format!("unknown tool '{tool_name}' in '{s}'");
        }
    }
    format!("malformed permission string '{s}'")
}

/// Parse the `path_access` config block.
///
/// Structure:
/// ```json
/// {
///   "allow": ["~/.config/**", "/tmp/**"],
///   "deny": ["/etc/**", "/usr/**"],
///   "ask": true
/// }
/// ```
///
/// Path glob patterns without a tool prefix apply to all tools.
/// If `ask: true`, a catch-all Ask rule is appended so that paths
/// not matching any allow/deny rule will prompt the user.
pub fn parse_path_access_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    // Deny rules first (deny beats allow within a source)
    if let Some(deny_arr) = config.get("deny").and_then(|v| v.as_array()) {
        for entry in deny_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.to_string()),
                    command_pattern: None,
                    operation: None,
                    action: RuleAction::Deny,
                });
            }
        }
    }

    // Allow rules second
    if let Some(allow_arr) = config.get("allow").and_then(|v| v.as_array()) {
        for entry in allow_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.to_string()),
                    command_pattern: None,
                    operation: None,
                    action: RuleAction::Allow,
                });
            }
        }
    }

    // If ask: true, append a catch-all Ask rule
    if let Some(ask) = config.get("ask").and_then(|v| v.as_bool()) {
        if ask {
            rules.push(PermissionRule {
                tool_pattern: None,
                path_pattern: Some("**".to_string()),
                command_pattern: None,
                operation: None,
                action: RuleAction::Ask,
            });
        }
    }

    Ok(rules)
}

/// Parse the `command_gate` config block.
///
/// Structure:
/// ```json
/// {
///   "patterns": {
///     "rm -rf": "ask",
///     "curl *": "deny"
///   },
///   "allow": ["git status", "git log"],
///   "deny": ["git *", "gh *"],
///   "ask": [],
///   "allow_git_in_bash": false
/// }
/// ```
///
/// Each key in `patterns` is a command substring pattern, each value is the
/// action (`"ask"` or `"deny"`).
///
/// The `allow`, `deny`, and `ask` arrays contain command patterns. Rules are
/// generated with no tool pattern (applies to all tools via the command gate).
///
/// Deny rules come first, then patterns (ask/deny from `patterns` object),
/// then allow rules — so deny takes precedence over allow within this block.
pub fn parse_command_gate_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    // Deny array first (deny beats allow within a source)
    if let Some(deny_arr) = config.get("deny").and_then(|v| v.as_array()) {
        for entry in deny_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: None,
                    command_pattern: Some(pattern.to_string()),
                    operation: None,
                    action: RuleAction::Deny,
                });
            }
        }
    }

    // Legacy patterns object (ask/deny)
    if let Some(patterns) = config.get("patterns").and_then(|v| v.as_object()) {
        for (pattern, action_val) in patterns {
            let action_str = match action_val.as_str() {
                Some(s) => s,
                None => {
                    return Err(format!(
                        "patterns['{pattern}']: expected a string action, got {:?}",
                        action_val
                    ));
                }
            };

            let action = match action_str {
                "ask" => RuleAction::Ask,
                "deny" => RuleAction::Deny,
                _ => {
                    return Err(format!(
                        "patterns['{pattern}']: unknown action '{action_str}', expected 'ask' or 'deny'"
                    ));
                }
            };

            rules.push(PermissionRule {
                tool_pattern: None,
                path_pattern: None,
                command_pattern: Some(pattern.clone()),
                operation: None,
                action,
            });
        }
    }

    // Ask array
    if let Some(ask_arr) = config.get("ask").and_then(|v| v.as_array()) {
        for entry in ask_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: None,
                    command_pattern: Some(pattern.to_string()),
                    operation: None,
                    action: RuleAction::Ask,
                });
            }
        }
    }

    // Allow array last
    if let Some(allow_arr) = config.get("allow").and_then(|v| v.as_array()) {
        for entry in allow_arr {
            if let Some(pattern) = entry.as_str() {
                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: None,
                    command_pattern: Some(pattern.to_string()),
                    operation: None,
                    action: RuleAction::Allow,
                });
            }
        }
    }

    Ok(rules)
}

/// Parse extra metadata from the `command_gate` config block (non-rule fields).
///
/// Currently extracts:
/// - `allow_git_in_bash: bool` — defaults to `false`
pub fn parse_command_gate_extras(config: &Value) -> CommandGateExtras {
    let allow_git_in_bash = config
        .get("allow_git_in_bash")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    CommandGateExtras { allow_git_in_bash }
}

/// Return the allow rules that bypass the built-in VCS deny patterns.
///
/// These rules allow `git` and `gh` commands in bash when `allow_git_in_bash`
/// is enabled. They are prepended to the config rules so they take precedence
/// over the built-in dangerous patterns.
pub fn vcs_bypass_rules() -> Vec<PermissionRule> {
    vec![
        PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("git".to_string()),
            operation: None,
            action: RuleAction::Allow,
        },
        PermissionRule {
            tool_pattern: None,
            path_pattern: None,
            command_pattern: Some("gh".to_string()),
            operation: None,
            action: RuleAction::Allow,
        },
    ]
}

/// Parse the `file_protection` config block.
///
/// Structure:
/// ```json
/// {
///   "patterns": {
///     ".env*": { "read": "ask", "write": "deny" },
///     "*.pem": { "read": "ask", "write": "deny" }
///   }
/// }
/// ```
///
/// Each key is a file path glob pattern, each value is a map of
/// operation (`"read"` or `"write"`) to action (`"ask"` or `"deny"`).
pub fn parse_file_protection_block(config: &Value) -> Result<Vec<PermissionRule>, String> {
    let mut rules = Vec::new();

    if let Some(patterns) = config.get("patterns").and_then(|v| v.as_object()) {
        for (pattern, ops) in patterns {
            let ops_obj = match ops.as_object() {
                Some(o) => o,
                None => {
                    return Err(format!(
                        "patterns['{pattern}']: expected an object with operation->action mappings, got {:?}",
                        ops
                    ));
                }
            };

            for (op, action_val) in ops_obj {
                let action_str = match action_val.as_str() {
                    Some(s) => s,
                    None => {
                        return Err(format!(
                            "patterns['{pattern}']['{op}']: expected a string action, got {:?}",
                            action_val
                        ));
                    }
                };

                let action = match action_str {
                    "allow" => RuleAction::Allow,
                    "ask" => RuleAction::Ask,
                    "deny" => RuleAction::Deny,
                    _ => {
                        return Err(format!(
                            "patterns['{pattern}']['{op}']: unknown action '{action_str}', expected 'allow', 'ask', or 'deny'"
                        ));
                    }
                };

                rules.push(PermissionRule {
                    tool_pattern: None,
                    path_pattern: Some(pattern.clone()),
                    command_pattern: None,
                    operation: None,
                    action,
                });
            }
        }
    }

    Ok(rules)
}

/// Merge multiple rule lists by priority order.
///
/// The slice is expected to be in priority order (first = highest priority).
/// Rules are simply concatenated — during evaluation, the first matching
/// rule wins, so higher-priority sources' rules are checked first.
pub fn merge_rules(rules: &[Vec<PermissionRule>]) -> Vec<PermissionRule> {
    rules.iter().flat_map(|r| r.iter().cloned()).collect()
}
