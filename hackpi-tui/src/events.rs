use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;
use hackpi_guardrails::{GuardReason, PermissionDecision};

/// Severity level for diagnostics messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

impl std::fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "INFO"),
            Self::Warning => write!(f, "WARN"),
            Self::Error => write!(f, "ERR"),
        }
    }
}

/// Structured summary of a tool call, derived from the tool name and its
/// JSON input parameters. Enables rendering of semantic action cards with
/// operation-specific titles and targets (e.g. "read src/main.rs").
///
/// # Resilience
///
/// `from_params` never panics on malformed or missing input fields. Missing
/// values degrade gracefully to sensible defaults (empty strings, `None`).
#[derive(Debug, Clone)]
pub enum ToolSummary {
    Read {
        path: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    Edit {
        path: String,
        operations: usize,
    },
    Write {
        path: String,
    },
    Search {
        pattern: String,
        path: Option<String>,
    },
    Bash {
        command: String,
    },
    Git {
        operation: String,
    },
    Github {
        operation: String,
    },
    Task {
        command: String,
    },
    Unknown,
}

impl ToolSummary {
    /// Build a `ToolSummary` from a tool name and optional JSON input.
    ///
    /// Handles schema hallucinations gracefully: missing fields produce
    /// sensible defaults, and unrecognised tool names produce `Unknown`.
    pub fn from_params(name: &str, input: Option<&serde_json::Value>) -> Self {
        match name {
            "read" => Self::from_read(input),
            "edit" => Self::from_edit(input),
            "write" => Self::from_write(input),
            "search_grep" | "search" | "grep" => Self::from_search(input),
            "bash" => Self::from_bash(input),
            "git_read" | "git_write" => Self::from_git(input),
            "github" => Self::from_github(input),
            "task" => Self::from_task(input),
            _ => Self::Unknown,
        }
    }

    /// Human-readable one-line title for the tool call card.
    ///
    /// Examples: `"read  src/main.rs"`, `"bash  cargo test"`, `"git  status"`
    pub fn title(&self) -> String {
        match self {
            Self::Read { path, .. } => {
                if path.is_empty() {
                    "read".into()
                } else {
                    format!("read  {path}")
                }
            }
            Self::Edit { path, operations } => {
                if path.is_empty() {
                    "edit".into()
                } else {
                    format!(
                        "edit  {path}  ({operations} op{})",
                        if *operations == 1 { "" } else { "s" }
                    )
                }
            }
            Self::Write { path } => {
                if path.is_empty() {
                    "write".into()
                } else {
                    format!("write  {path}")
                }
            }
            Self::Search { pattern, .. } => {
                if pattern.is_empty() {
                    "search".into()
                } else {
                    format!("search  {pattern}")
                }
            }
            Self::Bash { command } => {
                if command.is_empty() {
                    "bash".into()
                } else {
                    // Truncate long commands for display
                    let display = if command.len() > 60 {
                        format!("{}…", &command[..57])
                    } else {
                        command.clone()
                    };
                    format!("bash  {display}")
                }
            }
            Self::Git { operation } => {
                if operation.is_empty() {
                    "git".into()
                } else {
                    format!("git  {operation}")
                }
            }
            Self::Github { operation } => {
                if operation.is_empty() {
                    "github".into()
                } else {
                    format!("github  {operation}")
                }
            }
            Self::Task { command } => {
                if command.is_empty() {
                    "task".into()
                } else {
                    format!("task  {command}")
                }
            }
            Self::Unknown => "tool".into(),
        }
    }

    /// The primary target (file path, command, pattern) of the tool call,
    /// or `None` if no target is available.
    pub fn target(&self) -> Option<String> {
        match self {
            Self::Read { path, .. } => optional_nonempty(path),
            Self::Edit { path, .. } => optional_nonempty(path),
            Self::Write { path } => optional_nonempty(path),
            Self::Search { pattern, path } => {
                // Return the pattern if non-empty, falling back to the search path
                optional_nonempty(pattern).or_else(|| path.clone())
            }
            Self::Bash { command } => optional_nonempty(command),
            Self::Git { operation } => optional_nonempty(operation),
            Self::Github { operation } => optional_nonempty(operation),
            Self::Task { command } => optional_nonempty(command),
            Self::Unknown => None,
        }
    }

    /// The tool kind label (e.g. "read", "edit", "bash") for type-based styling.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Edit { .. } => "edit",
            Self::Write { .. } => "write",
            Self::Search { .. } => "search",
            Self::Bash { .. } => "bash",
            Self::Git { .. } => "git",
            Self::Github { .. } => "github",
            Self::Task { .. } => "task",
            Self::Unknown => "tool",
        }
    }

    // ── Per-tool constructors (private) ──────────────────────────────

    fn from_read(input: Option<&serde_json::Value>) -> Self {
        let input = match input.and_then(|v| v.as_object()) {
            Some(obj) => obj,
            None => {
                return Self::Read {
                    path: String::new(),
                    offset: None,
                    limit: None,
                }
            }
        };
        Self::Read {
            path: input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            offset: input
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|u| u as usize),
            limit: input
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|u| u as usize),
        }
    }

    fn from_edit(input: Option<&serde_json::Value>) -> Self {
        let input = match input.and_then(|v| v.as_object()) {
            Some(obj) => obj,
            None => {
                return Self::Edit {
                    path: String::new(),
                    operations: 0,
                }
            }
        };
        let path = input
            .get("filePath")
            .or_else(|| input.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // Count edit operations — can be "oldString"/"newString" pairs or an array of edits
        let operations = if let Some(ops) = input.get("operations").and_then(|v| v.as_array()) {
            ops.len()
        } else if input.contains_key("oldString") || input.contains_key("newString") {
            1
        } else {
            0
        };
        Self::Edit { path, operations }
    }

    fn from_write(input: Option<&serde_json::Value>) -> Self {
        let path = input
            .and_then(|v| v.as_object())
            .and_then(|obj| {
                obj.get("filePath")
                    .or_else(|| obj.get("path"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .to_string();
        Self::Write { path }
    }

    fn from_search(input: Option<&serde_json::Value>) -> Self {
        let input = match input.and_then(|v| v.as_object()) {
            Some(obj) => obj,
            None => {
                return Self::Search {
                    pattern: String::new(),
                    path: None,
                }
            }
        };
        Self::Search {
            pattern: input
                .get("pattern")
                .or_else(|| input.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            path: input
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        }
    }

    fn from_bash(input: Option<&serde_json::Value>) -> Self {
        let command = input
            .and_then(|v| v.as_object())
            .and_then(|obj| {
                obj.get("command")
                    .or_else(|| obj.get("cmd"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .to_string();
        Self::Bash { command }
    }

    fn from_git(input: Option<&serde_json::Value>) -> Self {
        let operation = input
            .and_then(|v| v.as_object())
            .and_then(|obj| obj.get("operation").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        Self::Git { operation }
    }

    fn from_github(input: Option<&serde_json::Value>) -> Self {
        let operation = input
            .and_then(|v| v.as_object())
            .and_then(|obj| obj.get("operation").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        Self::Github { operation }
    }

    fn from_task(input: Option<&serde_json::Value>) -> Self {
        let command = input
            .and_then(|v| v.as_object())
            .and_then(|obj| {
                obj.get("command")
                    .or_else(|| obj.get("subcommand"))
                    .or_else(|| obj.get("operation"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .to_string();
        Self::Task { command }
    }
}

/// Return `Some(s)` if `s` is non-empty, else `None`.
fn optional_nonempty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[derive(Debug)]
pub enum TuiEvent {
    Submit(String),
    StreamChunk(String),
    ToolCall {
        id: String,
        name: String,
        /// Raw JSON input parameters for deriving a structured summary.
        /// Set to `None` when the input is not available (e.g. legacy event sources).
        input: Option<serde_json::Value>,
    },
    ToolResult {
        id: String,
        result: ToolResult,
    },
    Error(String),
    /// A protocol-level diagnostic message (SSE parse failure, stream
    /// truncation warning, etc.) that should be stored in the diagnostics
    /// store rather than rendered as a conversation entry.
    Diagnostic {
        level: DiagnosticLevel,
        message: String,
    },
    Usage(Usage),
    Done,
    PermissionRequest {
        id: u64,
        reason: GuardReason,
        response: tokio::sync::oneshot::Sender<PermissionDecision>,
    },
}

impl TuiEvent {
    /// Serialize this event as a JSON line for structured/machine-readable output.
    ///
    /// Returns `None` for events that cannot be serialized (e.g. `PermissionRequest`
    /// which contains a non-serializable oneshot sender).
    ///
    /// Each returned JSON object includes a `type` field identifying the event kind
    /// and a `timestamp` field in ISO 8601 format.
    pub fn to_json_line(&self) -> Option<String> {
        fn timestamp() -> String {
            use chrono::Utc;
            Utc::now().to_rfc3339()
        }

        let ts = timestamp();
        match self {
            TuiEvent::Submit(text) => Some(serde_json::json!({
                "type": "submit",
                "text": text,
                "timestamp": ts,
            })),
            TuiEvent::StreamChunk(text) => Some(serde_json::json!({
                "type": "stream_chunk",
                "text": text,
                "timestamp": ts,
            })),
            TuiEvent::ToolCall { id, name, .. } => Some(serde_json::json!({
                "type": "tool_call",
                "id": id,
                "name": name,
                "timestamp": ts,
            })),
            TuiEvent::ToolResult { id, result } => {
                let status = match result {
                    ToolResult::Success { .. } => "success",
                    ToolResult::SystemError { .. } => "error",
                    ToolResult::Timeout => "timeout",
                    ToolResult::Cancelled => "cancelled",
                };
                Some(serde_json::json!({
                    "type": "tool_result",
                    "id": id,
                    "status": status,
                    "timestamp": ts,
                }))
            }
            TuiEvent::Error(msg) => Some(serde_json::json!({
                "type": "error",
                "message": msg,
                "timestamp": ts,
            })),
            TuiEvent::Diagnostic { level, message } => Some(serde_json::json!({
                "type": "diagnostic",
                "level": level.to_string(),
                "message": message,
                "timestamp": ts,
            })),
            TuiEvent::Usage(u) => Some(serde_json::json!({
                "type": "usage",
                "input_tokens": u.input_tokens,
                "output_tokens": u.output_tokens,
                "timestamp": ts,
            })),
            TuiEvent::Done => Some(serde_json::json!({
                "type": "done",
                "timestamp": ts,
            })),
            TuiEvent::PermissionRequest { .. } => None,
        }
        .map(|v| v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ToolSummary::from_params — read ──────────────────────────────

    #[test]
    fn test_summary_read_with_path_and_offset() {
        let input = serde_json::json!({"path": "src/main.rs", "offset": 10, "limit": 50});
        let summary = ToolSummary::from_params("read", Some(&input));
        assert_eq!(summary.title(), "read  src/main.rs");
        assert_eq!(summary.target(), Some("src/main.rs".into()));
        assert_eq!(summary.kind(), "read");
        if let ToolSummary::Read {
            path,
            offset,
            limit,
        } = &summary
        {
            assert_eq!(path, "src/main.rs");
            assert_eq!(*offset, Some(10));
            assert_eq!(*limit, Some(50));
        } else {
            panic!("expected Read variant");
        }
    }

    #[test]
    fn test_summary_read_minimal() {
        let input = serde_json::json!({"path": "Cargo.toml"});
        let summary = ToolSummary::from_params("read", Some(&input));
        assert_eq!(summary.title(), "read  Cargo.toml");
        if let ToolSummary::Read { offset, limit, .. } = &summary {
            assert_eq!(*offset, None);
            assert_eq!(*limit, None);
        } else {
            panic!("expected Read variant");
        }
    }

    #[test]
    fn test_summary_read_no_input() {
        let summary = ToolSummary::from_params("read", None);
        assert_eq!(summary.title(), "read");
        assert_eq!(summary.target(), None);
    }

    #[test]
    fn test_summary_read_non_object_input() {
        let input = serde_json::json!("just a string");
        let summary = ToolSummary::from_params("read", Some(&input));
        assert_eq!(summary.title(), "read");
        assert_eq!(summary.target(), None);
    }

    // ── ToolSummary::from_params — edit ──────────────────────────────

    #[test]
    fn test_summary_edit_with_path() {
        let input =
            serde_json::json!({"filePath": "src/main.rs", "oldString": "foo", "newString": "bar"});
        let summary = ToolSummary::from_params("edit", Some(&input));
        assert!(summary.title().contains("src/main.rs"));
        assert!(summary.title().contains("1 op"));
        assert_eq!(summary.target(), Some("src/main.rs".into()));
    }

    #[test]
    fn test_summary_edit_with_path_alias() {
        let input = serde_json::json!({"path": "src/lib.rs", "oldString": "x", "newString": "y"});
        let summary = ToolSummary::from_params("edit", Some(&input));
        assert!(summary.title().contains("src/lib.rs"));
    }

    #[test]
    fn test_summary_edit_with_operations_array() {
        let input = serde_json::json!({"filePath": "src/main.rs", "operations": [{"old": "a", "new": "b"}, {"old": "c", "new": "d"}]});
        let summary = ToolSummary::from_params("edit", Some(&input));
        assert!(summary.title().contains("2 ops"));
    }

    #[test]
    fn test_summary_edit_no_path() {
        let input = serde_json::json!({"oldString": "foo"});
        let summary = ToolSummary::from_params("edit", Some(&input));
        assert_eq!(summary.title(), "edit");
        assert_eq!(summary.target(), None);
    }

    #[test]
    fn test_summary_edit_no_input() {
        let summary = ToolSummary::from_params("edit", None);
        assert_eq!(summary.title(), "edit");
        assert_eq!(summary.kind(), "edit");
    }

    // ── ToolSummary::from_params — write ─────────────────────────────

    #[test]
    fn test_summary_write() {
        let input = serde_json::json!({"filePath": "src/output.txt"});
        let summary = ToolSummary::from_params("write", Some(&input));
        assert_eq!(summary.title(), "write  src/output.txt");
        assert_eq!(summary.target(), Some("src/output.txt".into()));
    }

    #[test]
    fn test_summary_write_no_input() {
        let summary = ToolSummary::from_params("write", None);
        assert_eq!(summary.title(), "write");
    }

    // ── ToolSummary::from_params — search ────────────────────────────

    #[test]
    fn test_summary_search_with_pattern() {
        let input = serde_json::json!({"pattern": "fn main", "path": "src/"});
        let summary = ToolSummary::from_params("search_grep", Some(&input));
        assert_eq!(summary.title(), "search  fn main");
        assert_eq!(summary.target(), Some("fn main".into()));
        assert_eq!(summary.kind(), "search");
        if let ToolSummary::Search { pattern, path } = &summary {
            assert_eq!(pattern, "fn main");
            assert_eq!(path.as_deref(), Some("src/"));
        } else {
            panic!("expected Search variant");
        }
    }

    #[test]
    fn test_summary_search_with_query_alias() {
        let input = serde_json::json!({"query": "TODO"});
        let summary = ToolSummary::from_params("search", Some(&input));
        assert_eq!(summary.title(), "search  TODO");
    }

    #[test]
    fn test_summary_search_no_input() {
        let summary = ToolSummary::from_params("grep", None);
        assert_eq!(summary.title(), "search");
    }

    // ── ToolSummary::from_params — bash ──────────────────────────────

    #[test]
    fn test_summary_bash() {
        let input = serde_json::json!({"command": "cargo test"});
        let summary = ToolSummary::from_params("bash", Some(&input));
        assert!(summary.title().contains("cargo test"));
        assert_eq!(summary.target(), Some("cargo test".into()));
        assert_eq!(summary.kind(), "bash");
    }

    #[test]
    fn test_summary_bash_with_cmd_alias() {
        let input = serde_json::json!({"cmd": "ls -la"});
        let summary = ToolSummary::from_params("bash", Some(&input));
        assert!(summary.title().contains("ls -la"));
    }

    #[test]
    fn test_summary_bash_long_command_truncated() {
        let long = "a".repeat(100);
        let input = serde_json::json!({"command": long});
        let summary = ToolSummary::from_params("bash", Some(&input));
        let title = summary.title();
        assert!(
            title.len() < 80,
            "long bash command should be truncated: {title}"
        );
        assert!(
            title.contains("…"),
            "truncated title should contain ellipsis"
        );
    }

    #[test]
    fn test_summary_bash_no_input() {
        let summary = ToolSummary::from_params("bash", None);
        assert_eq!(summary.title(), "bash");
    }

    // ── ToolSummary::from_params — git ───────────────────────────────

    #[test]
    fn test_summary_git() {
        let input = serde_json::json!({"operation": "status"});
        let summary = ToolSummary::from_params("git_read", Some(&input));
        assert_eq!(summary.title(), "git  status");
        assert_eq!(summary.target(), Some("status".into()));
    }

    #[test]
    fn test_summary_git_no_operation() {
        let input = serde_json::json!({});
        let summary = ToolSummary::from_params("git_write", Some(&input));
        assert_eq!(summary.title(), "git");
        assert_eq!(summary.target(), None);
    }

    // ── ToolSummary::from_params — github ────────────────────────────

    #[test]
    fn test_summary_github() {
        let input = serde_json::json!({"operation": "pr_list"});
        let summary = ToolSummary::from_params("github", Some(&input));
        assert_eq!(summary.title(), "github  pr_list");
        assert_eq!(summary.target(), Some("pr_list".into()));
    }

    // ── ToolSummary::from_params — task ──────────────────────────────

    #[test]
    fn test_summary_task() {
        let input = serde_json::json!({"command": "list"});
        let summary = ToolSummary::from_params("task", Some(&input));
        assert_eq!(summary.title(), "task  list");
    }

    #[test]
    fn test_summary_task_with_subcommand() {
        let input = serde_json::json!({"subcommand": "create", "title": "Fix bug"});
        let summary = ToolSummary::from_params("task", Some(&input));
        assert_eq!(summary.title(), "task  create");
    }

    // ── ToolSummary::from_params — unknown ───────────────────────────

    #[test]
    fn test_summary_unknown_tool() {
        let summary = ToolSummary::from_params("nonexistent", None);
        assert_eq!(summary.title(), "tool");
        assert_eq!(summary.target(), None);
        assert_eq!(summary.kind(), "tool");
    }

    #[test]
    fn test_summary_unknown_tool_with_input() {
        let input = serde_json::json!({"foo": "bar"});
        let summary = ToolSummary::from_params("weird_tool", Some(&input));
        assert_eq!(summary.title(), "tool");
    }

    // ── ToolSummary edge cases ───────────────────────────────────────

    #[test]
    fn test_summary_null_input() {
        let summary = ToolSummary::from_params("read", Some(&serde_json::Value::Null));
        assert_eq!(summary.title(), "read");
    }

    #[test]
    fn test_summary_array_input() {
        let input = serde_json::json!([1, 2, 3]);
        let summary = ToolSummary::from_params("bash", Some(&input));
        assert_eq!(summary.title(), "bash");
    }

    #[test]
    fn test_summary_empty_object() {
        let input = serde_json::json!({});
        let summary = ToolSummary::from_params("read", Some(&input));
        assert_eq!(summary.title(), "read");
        if let ToolSummary::Read { path, .. } = &summary {
            assert_eq!(path, "");
        } else {
            panic!("expected Read variant");
        }
    }

    // ── TuiEvent::to_json_line tests ───────────────────────────────────

    #[test]
    fn test_to_json_line_submit() {
        let event = TuiEvent::Submit("hello".into());
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"submit\""));
        assert!(json.contains("\"text\":\"hello\""));
        assert!(json.contains("\"timestamp\":"));
    }

    #[test]
    fn test_to_json_line_stream_chunk() {
        let event = TuiEvent::StreamChunk("some text".into());
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"stream_chunk\""));
        assert!(json.contains("\"text\":\"some text\""));
    }

    #[test]
    fn test_to_json_line_tool_call() {
        let event = TuiEvent::ToolCall {
            id: "tc-1".into(),
            name: "bash".into(),
            input: None,
        };
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"tool_call\""));
        assert!(json.contains("\"id\":\"tc-1\""));
        assert!(json.contains("\"name\":\"bash\""));
    }

    #[test]
    fn test_to_json_line_tool_result_success() {
        let event = TuiEvent::ToolResult {
            id: "tc-1".into(),
            result: ToolResult::Success {
                content: "ok".into(),
            },
        };
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"id\":\"tc-1\""));
        assert!(json.contains("\"status\":\"success\""));
    }

    #[test]
    fn test_to_json_line_tool_result_error() {
        let event = TuiEvent::ToolResult {
            id: "tc-2".into(),
            result: ToolResult::SystemError {
                message: "boom".into(),
            },
        };
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"status\":\"error\""));
    }

    #[test]
    fn test_to_json_line_tool_result_timeout() {
        let event = TuiEvent::ToolResult {
            id: "tc-3".into(),
            result: ToolResult::Timeout,
        };
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"status\":\"timeout\""));
    }

    #[test]
    fn test_to_json_line_tool_result_cancelled() {
        let event = TuiEvent::ToolResult {
            id: "tc-4".into(),
            result: ToolResult::Cancelled,
        };
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"status\":\"cancelled\""));
    }

    #[test]
    fn test_to_json_line_error() {
        let event = TuiEvent::Error("something went wrong".into());
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("\"message\":\"something went wrong\""));
    }

    #[test]
    fn test_to_json_line_usage() {
        let event = TuiEvent::Usage(Usage {
            input_tokens: 100,
            output_tokens: 50,
        });
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"usage\""));
        assert!(json.contains("\"input_tokens\":100"));
        assert!(json.contains("\"output_tokens\":50"));
    }

    #[test]
    fn test_to_json_line_done() {
        let event = TuiEvent::Done;
        let json = event.to_json_line().expect("should produce JSON");
        assert!(json.contains("\"type\":\"done\""));
    }

    #[test]
    fn test_to_json_line_permission_request_returns_none() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let event = TuiEvent::PermissionRequest {
            id: 1,
            reason: hackpi_guardrails::GuardReason {
                guard: hackpi_guardrails::GuardType::CommandGate,
                tool: "bash".into(),
                details: "test".into(),
            },
            response: tx,
        };
        assert!(event.to_json_line().is_none());
    }

    #[test]
    fn test_to_json_line_output_is_valid_json() {
        let event = TuiEvent::Submit("test".into());
        let json = event.to_json_line().expect("should produce JSON");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should be valid JSON");
        assert_eq!(parsed["type"], "submit");
        assert_eq!(parsed["text"], "test");
        assert!(parsed["timestamp"].is_string());
    }
}
