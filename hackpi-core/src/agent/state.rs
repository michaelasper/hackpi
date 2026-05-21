use crate::api::ApiClient;
use crate::tools::{ToolRegistry, ToolResult};
use crate::types::Usage;
use hackpi_guardrails::{GuardReason, PermissionDecision};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot;

pub(crate) const MAX_TURNS: u32 = 25;
pub(crate) const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;

pub(crate) fn truncate_output(
    content: &str,
    max_bytes: usize,
    tool_id: &str,
    _workspace_root: &Path,
) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    let safe_id: String = tool_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let safe_id = if safe_id.is_empty() {
        "unknown"
    } else {
        &safe_id
    };

    // Write full output to a temp file in /tmp so we don't pollute the workspace.
    // Include a timestamp for uniqueness across concurrent tool calls.
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path =
        std::path::PathBuf::from(format!("/tmp/hackpi-truncated-{safe_id}-{timestamp}.txt"));
    let write_ok = std::fs::write(&tmp_path, content).is_ok();

    let end = content.floor_char_boundary(max_bytes);
    let mut clipped = content[..end].to_string();
    if write_ok {
        clipped.push_str(&format!(
            "\n\n[Output truncated: {} total bytes. Full output written to {}]",
            content.len(),
            tmp_path.display()
        ));
    } else {
        clipped.push_str(&format!(
            "\n\n[Output truncated: {} total bytes. Could not write full output to disk.]",
            content.len(),
        ));
    }
    clipped
}

#[derive(Debug)]
pub enum AgentEvent {
    TextChunk(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallEnd {
        id: String,
        result: ToolResult,
    },
    Done,
    Error(String),
    Usage(Usage),
    PermissionRequest {
        id: u64,
        reason: GuardReason,
        response: oneshot::Sender<PermissionDecision>,
    },
}

/// A pending tool call collected during SSE event processing.
/// If `parse_error` is `Some`, the tool call's input JSON was malformed
/// and `input` will be `Value::Null`. The `execute_pending_tool_calls`
/// method will return a `ToolResult::SystemError` instead of dispatching.
pub(crate) struct PendingToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) input: Value,
    pub(crate) parse_error: Option<String>,
}

/// Result of processing SSE events from the API stream.
pub(crate) struct SseEvents {
    pub(crate) text: String,
    pub(crate) pending_tool_calls: Vec<PendingToolCall>,
    pub(crate) stop_reason: Option<String>,
    pub(crate) usage: Option<Usage>,
}

pub struct Agent {
    pub(crate) api: ApiClient,
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) system_prompt: String,
    pub(crate) workspace_root: PathBuf,
}

impl Agent {
    pub fn new(
        api: ApiClient,
        tools: Arc<ToolRegistry>,
        system_prompt: String,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            api,
            tools,
            system_prompt,
            workspace_root,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_output_under_limit_passthrough() {
        let content = "hello world";
        let result = truncate_output(content, 1024, "tool_1", std::path::Path::new("/tmp"));

        assert_eq!(result, content);
    }

    #[test]
    fn test_truncate_output_at_exact_limit_passthrough() {
        let content = "a".repeat(100);
        let result = truncate_output(&content, 100, "tool_1", std::path::Path::new("/tmp"));

        assert_eq!(
            result, content,
            "content at exact limit should pass through"
        );
    }

    #[test]
    fn test_truncate_output_one_byte_over_truncates() {
        let content = "a".repeat(101);
        let result = truncate_output(&content, 100, "tool_1", std::path::Path::new("/tmp"));

        assert_ne!(result, content, "content over limit should be modified");
        assert!(
            result.contains("Output truncated"),
            "should mention truncation"
        );
    }

    #[test]
    fn test_truncate_output_over_limit_writes_full_content_to_disk() {
        let tool_id = "disk_test_unique";
        let content = "a".repeat(1000);
        let _result = truncate_output(&content, 100, tool_id, std::path::Path::new("/tmp"));

        // Find the temp file in /tmp with the expected pattern
        let entries = std::fs::read_dir("/tmp").unwrap();
        let tmp_file = entries
            .filter_map(|e| e.ok())
            .find(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| {
                        s.starts_with("hackpi-truncated-disk_test_unique-") && s.ends_with(".txt")
                    })
                    .unwrap_or(false)
            })
            .expect("temp file should exist in /tmp");
        let on_disk = std::fs::read_to_string(tmp_file.path()).unwrap();
        assert_eq!(on_disk, content, "temp file should contain full output");

        // Clean up
        let _ = std::fs::remove_file(tmp_file.path());
    }

    #[test]
    fn test_truncate_output_over_limit_clips_and_mentions_file() {
        let tool_id = "clip_test";
        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, tool_id, std::path::Path::new("/tmp"));

        assert!(result.len() < content.len());
        let expected_clip: String = "a".repeat(100);
        assert!(result.starts_with(&expected_clip));
        assert!(result.contains("Output truncated"));
        assert!(result.contains("/tmp/hackpi-truncated-clip_test-"));

        // Clean up temp file
        let entries = std::fs::read_dir("/tmp").unwrap();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-clip_test-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_sanitizes_tool_id() {
        let content = "a".repeat(1000);
        let result = truncate_output(
            &content,
            100,
            "../../etc/passwd",
            std::path::Path::new("/tmp"),
        );

        assert!(
            !result.contains("../../etc/passwd"),
            "path traversal chars should be removed"
        );
        assert!(result.contains("/tmp/hackpi-truncated-"), "safe path used");
        // After filtering "../../etc/passwd", remaining chars are "etcpasswd"
        assert!(
            result.contains("etcpasswd"),
            "sanitized tool_id should appear in message"
        );

        // Verify the file exists in /tmp
        let entries = std::fs::read_dir("/tmp").unwrap();
        let found = entries.flatten().any(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("hackpi-truncated-etcpasswd-") && name.ends_with(".txt")
        });
        assert!(found, "file should use sanitized tool_id chars in /tmp");

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-etcpasswd-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_unsafe_only_tool_id_falls_back() {
        let content = "a".repeat(1000);
        // tool_id with ONLY unsafe chars (dots and slashes)
        let result = truncate_output(&content, 100, "../", std::path::Path::new("/tmp"));

        assert!(
            !result.contains("../"),
            "unsafe-only tool_id should be replaced"
        );
        assert!(result.contains("unknown"), "should use 'unknown' fallback");
        assert!(
            result.contains("/tmp/hackpi-truncated-unknown-"),
            "should use 'unknown' fallback in path"
        );

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-unknown-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_empty_tool_id_uses_fallback() {
        let content = "a".repeat(1000);
        let result = truncate_output(&content, 100, "", std::path::Path::new("/tmp"));

        assert!(result.contains("unknown"));

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-unknown-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    #[test]
    fn test_truncate_output_non_ascii_safe_boundary() {
        // multi-byte UTF-8 chars: each 'é' is 2 bytes
        let content = "é".repeat(200);
        let result = truncate_output(&content, 100, "utf8_test", std::path::Path::new("/tmp"));

        // 100 bytes should cut at char boundary (floor to even: 100)
        assert!(
            result.contains("Output truncated"),
            "should truncate safely at char boundary"
        );
        // Should not panic from mid-char split

        // Clean up
        for entry in std::fs::read_dir("/tmp").unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hackpi-truncated-utf8_test-") && name.ends_with(".txt") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
