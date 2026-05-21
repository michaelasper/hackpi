use crate::RuleAction;

/// A built-in dangerous command pattern with its action.
pub struct DangerousPattern {
    pub pattern: &'static str,
    pub action: RuleAction,
    /// If true, the pattern must match at word boundaries (no partial word matches).
    pub word_boundary: bool,
    /// If true, matching is case-sensitive. Default (false) is case-insensitive.
    pub case_sensitive: bool,
    /// If set, this pattern only applies to commands from this specific tool
    /// (e.g., "bash"). If None, applies to all tools.
    pub tool_scope: Option<&'static str>,
}

/// Built-in dangerous command patterns checked as a fallback after config rules.
///
/// More specific patterns must come before less specific ones since
/// `check_against_dangerous_patterns` returns the first match.
pub const DANGEROUS_PATTERNS: &[DangerousPattern] = &[
    // Deny patterns first (fail-closed, highest severity)
    DangerousPattern {
        pattern: ":(){ :|:& };:",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "> /dev/sda",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "> /dev/nvme",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    // No-space redirect variants
    DangerousPattern {
        pattern: ">/dev/sda",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: ">/dev/nvme",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "mkfs.",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "dd",
        action: RuleAction::Deny,
        word_boundary: true,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "sudo",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "su",
        action: RuleAction::Deny,
        word_boundary: true,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "doas",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "passwd",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "chpasswd",
        action: RuleAction::Deny,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    // VCS commands — use dedicated git_read/git_write/github tools instead
    // Only applies to the "bash" tool so VCS tools are not blocked.
    DangerousPattern {
        pattern: "git",
        action: RuleAction::Deny,
        word_boundary: true,
        case_sensitive: false,
        tool_scope: Some("bash"),
    },
    DangerousPattern {
        pattern: "gh",
        action: RuleAction::Deny,
        word_boundary: true,
        case_sensitive: false,
        tool_scope: Some("bash"),
    },
    // Ask patterns second (notable but potentially legitimate)
    DangerousPattern {
        pattern: "rm -rf",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "rm -r",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "chmod -R",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: true,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "chown -R",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "curl",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
    DangerousPattern {
        pattern: "wget",
        action: RuleAction::Ask,
        word_boundary: false,
        case_sensitive: false,
        tool_scope: None,
    },
];
