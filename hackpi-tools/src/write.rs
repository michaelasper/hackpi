use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use tokio::fs;

pub struct WriteTool {
    workspace_root: std::path::PathBuf,
}

impl WriteTool {
    pub fn new(workspace_root: std::path::PathBuf) -> Self {
        Self { workspace_root }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Creates a completely new file at the specified path with the provided content. \
         CRITICAL: This tool will hard-fail if the file already exists. \
         To modify existing files, you MUST use the edit tool instead."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The absolute or relative path where the new file should be created \
                                   (e.g., 'src/agent/orchestrator.rs'). Parent directories will be \
                                   created automatically if they do not exist."
                },
                "content": {
                    "type": "string",
                    "description": "The complete, raw text content to write to the new file. \
                                   Do not wrap in markdown code blocks unless the file itself requires it."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match params.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'path' parameter.".into(),
                }
            }
        };

        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'content' parameter.".into(),
                }
            }
        };

        let canonical =
            match crate::path_jail::resolve_workspace_path(&self.workspace_root, file_path) {
                Ok(p) => p,
                Err(e) => return e,
            };

        // Overwrite trap
        if canonical.exists() {
            return ToolResult::SystemError {
                message: "Error: File already exists. Use edit to modify.".into(),
            };
        }

        // Phantom directory handler
        if let Some(parent) = canonical.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent).await {
                    return ToolResult::SystemError {
                        message: format!(
                            "Failed to create parent directories for {file_path}: {e}"
                        ),
                    };
                }
            }
        }

        // Atomic write: temp file then rename
        let file_name = canonical
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let tmp_path = canonical.with_file_name(format!(".{file_name}.tmp"));

        if let Err(e) = fs::write(&tmp_path, content.as_bytes()).await {
            let msg = match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    format!("Permission denied: {file_path}")
                }
                _ => format!("IO error: {e}"),
            };
            return ToolResult::SystemError { message: msg };
        }

        if let Err(e) = fs::rename(&tmp_path, &canonical).await {
            let _ = fs::remove_file(&tmp_path).await;
            let msg = match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    format!("Permission denied: {file_path}")
                }
                _ => format!("IO error: {e}"),
            };
            return ToolResult::SystemError { message: msg };
        }

        let byte_count = content.len();
        let line_count = content.lines().count();

        ToolResult::Success {
            content: format!("Wrote {file_path}: {byte_count} bytes, {line_count} lines"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn temp_dir() -> std::path::PathBuf {
        static COUNTER: OnceLock<std::sync::atomic::AtomicU32> = OnceLock::new();
        let c = COUNTER.get_or_init(|| std::sync::atomic::AtomicU32::new(0));
        let id = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hackpi_write_test_{id}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn test_write_with_path_param_succeeds() {
        let dir = temp_dir();
        let tool = WriteTool::new(dir.clone());

        let params = serde_json::json!({
            "path": "hello.txt",
            "content": "Hello, world!"
        });
        let ctx = ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;

        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("hello.txt"),
                    "expected success mentioning file, got: {content}"
                );
            }
            other => panic!("expected Success with 'path' param, got {other:?}"),
        }

        let file_path = dir.join("hello.txt");
        assert!(file_path.exists(), "file should have been created");
        let contents = std::fs::read_to_string(&file_path).unwrap();
        assert_eq!(contents, "Hello, world!");
    }

    #[tokio::test]
    async fn test_write_with_old_file_path_param_fails() {
        let dir = temp_dir();
        let tool = WriteTool::new(dir.clone());

        let params = serde_json::json!({
            "filePath": "should_not_exist.txt",
            "content": "should not be written"
        });
        let ctx = ToolContext {
            workspace_root: dir.clone(),
            conversation_id: String::new(),
            signal: tokio::sync::watch::channel(false).1,
        };

        let result = tool.execute(params, &ctx).await;

        match result {
            ToolResult::SystemError { message } => {
                assert!(
                    message.contains("path"),
                    "error should mention 'path' parameter, got: {message}"
                );
            }
            other => panic!("expected SystemError for old 'filePath' param, got {other:?}"),
        }

        let file_path = dir.join("should_not_exist.txt");
        assert!(!file_path.exists(), "file should NOT have been created");
    }
}
