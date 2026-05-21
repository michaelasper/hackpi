use std::collections::VecDeque;

use crate::events::{DiagnosticLevel, ToolSummary};
use hackpi_core::tools::ToolResult;

use super::state::Severity;

/// The kind of a conversation entry — either a normal message or a system error.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationEntryKind {
    /// A regular user or assistant message.
    Message,
    /// A system error rendered inline in the conversation.
    SystemError {
        severity: Severity,
        recovery_hint: Option<String>,
    },
}

pub struct ConversationEntry {
    pub kind: ConversationEntryKind,
    pub role: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallDisplay>,
}

pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub summary: ToolSummary,
    pub status: ToolCallStatus,
}

pub enum ToolCallStatus {
    Running,
    Done(ToolResult),
}

/// A single diagnostics entry — a protocol-level log message (SSE parse
/// failure, stream truncation warning, etc.) that is stored separately from
/// the conversation viewport.
#[derive(Debug, Clone)]
pub struct DiagnosticsEntry {
    /// Severity level of the diagnostic.
    pub level: DiagnosticLevel,
    /// The diagnostic message text.
    pub message: String,
    /// ISO 8601 timestamp of when this diagnostic was recorded.
    pub timestamp: String,
}

impl DiagnosticsEntry {
    /// Create a new diagnostics entry with an auto-generated timestamp.
    pub fn new(level: DiagnosticLevel, message: impl Into<String>) -> Self {
        let now = chrono::Utc::now();
        Self {
            level,
            message: message.into(),
            timestamp: now.to_rfc3339(),
        }
    }
}

/// Format the conversation history as a markdown text document suitable for
/// LLM analysis. Includes a metadata header (date, message count) and each
/// conversation entry with role labels, timestamps, text content, and tool
/// call details (name, status, result).
///
/// The output is designed to be clean and parseable, with clear section
/// separators and consistent formatting across all entry types.
pub fn format_conversation(conversation: &VecDeque<ConversationEntry>) -> String {
    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut output = String::new();
    output.push_str("# HackPI Conversation Export\n\n");
    output.push_str(&format!("Date: {date_str}\n"));
    output.push_str(&format!("Messages: {}\n\n", conversation.len()));

    for (i, entry) in conversation.iter().enumerate() {
        let msg_num = i + 1;
        output.push_str(&format!("## Message {msg_num}\n"));
        output.push_str(&format!("**Role**: {}\n", entry.role));

        match &entry.kind {
            ConversationEntryKind::SystemError {
                severity,
                recovery_hint,
            } => {
                let severity_label = match severity {
                    Severity::Info => "Info",
                    Severity::Warning => "Warning",
                    Severity::Error => "Error",
                };
                output.push_str(&format!("**Type**: System Error ({severity_label})\n"));
                if let Some(hint) = recovery_hint {
                    output.push_str(&format!("**Recovery**: {hint}\n"));
                }
            }
            ConversationEntryKind::Message => {}
        }

        output.push_str("---\n");

        if !entry.text.is_empty() {
            output.push_str(&entry.text);
            output.push('\n');
        }

        if !entry.tool_calls.is_empty() {
            output.push('\n');
            for tc in &entry.tool_calls {
                let title = tc.summary.title();
                let status_str = match &tc.status {
                    ToolCallStatus::Running => "Running".to_string(),
                    ToolCallStatus::Done(result) => match result {
                        ToolResult::Success { content } => {
                            format!("Done (Success)\n\n```\n{content}\n```")
                        }
                        ToolResult::SystemError { message } => {
                            format!("Done (Error: {message})")
                        }
                        ToolResult::CommandError { content, exit_code } => {
                            format!("Done (Exit {exit_code})\n\n```\n{content}\n```")
                        }
                        ToolResult::Timeout => "Done (Timeout)".to_string(),
                        ToolResult::Cancelled => "Done (Cancelled)".to_string(),
                    },
                };
                output.push_str(&format!("### Tool: {title}\n"));
                output.push_str(&format!("**Tool**: {}\n", tc.name));
                output.push_str(&format!("**Status**: {status_str}\n\n"));
            }
        }

        output.push('\n');
    }

    output
}
