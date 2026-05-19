use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::Mutex;

use super::filesystem::InMemoryFs;
use super::session::{normalize_path, BashSession};

pub struct BashTool {
    #[allow(dead_code)]
    workspace_root: PathBuf,
    session: Mutex<BashSession>,
}

impl BashTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        let session = BashSession::new(Box::new(InMemoryFs::default()));
        Self {
            workspace_root,
            session: Mutex::new(session),
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in a persistent virtual shell. The filesystem persists across calls."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 120)."
                },
                "workdir": {
                    "type": "string",
                    "description": "Working directory override (absolute path within virtual fs)."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult {
        let command = match params.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'command' parameter.".into(),
                }
            }
        };

        let workdir = params.get("workdir").and_then(|v| v.as_str());
        let timeout_secs = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(30)
            .min(120);

        if *ctx.signal.borrow() {
            return ToolResult::Cancelled;
        }

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout_dur, async {
            let mut session = self.session.lock().await;
            session.signal = Some(ctx.signal.clone());
            if let Some(wd) = workdir {
                let normalized = normalize_path(wd);
                if session.fs.is_dir(std::path::Path::new(&normalized)) {
                    session.cwd = std::path::PathBuf::from(normalized);
                }
            }
            session.execute(command)
        })
        .await;

        let output = match result {
            Ok(out) => out,
            Err(_) => return ToolResult::Timeout,
        };

        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&output.stdout);
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&output.stderr);
        }
        if output.exit_code != 0 && result.is_empty() {
            result = format!("Command exited with code {}", output.exit_code);
        }

        ToolResult::Success { content: result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_core::tools::ToolContext;

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        let schema = tool.input_schema();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&serde_json::json!(false)),
            "bash tool schema missing additionalProperties: false"
        );
    }

    #[tokio::test]
    async fn test_bash_execute_echo() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        let params = serde_json::json!({
            "command": "echo hello world"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        match result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("hello world"),
                    "expected 'hello world' in output, got: {content}"
                );
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }
}
