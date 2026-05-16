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
                "filePath": {
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
            "required": ["filePath", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let file_path = match params.get("filePath").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'filePath' parameter.".into(),
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

        let path = self.workspace_root.join(file_path);

        // Path jail: prevent writing outside workspace
        let canonical = std::fs::canonicalize(&path).unwrap_or(path.clone());
        if !canonical.starts_with(&self.workspace_root) {
            return ToolResult::SystemError {
                message: "Security Error: Attempted to write outside workspace.".into(),
            };
        }

        // Overwrite trap
        if path.exists() {
            return ToolResult::SystemError {
                message: "Error: File already exists. Use edit to modify.".into(),
            };
        }

        // Phantom directory handler
        if let Some(parent) = path.parent() {
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
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let tmp_path = path.with_file_name(format!(".{file_name}.tmp"));

        if let Err(e) = fs::write(&tmp_path, content.as_bytes()).await {
            let msg = match e.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    format!("Permission denied: {file_path}")
                }
                _ => format!("IO error: {e}"),
            };
            return ToolResult::SystemError { message: msg };
        }

        if let Err(e) = fs::rename(&tmp_path, &path).await {
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
