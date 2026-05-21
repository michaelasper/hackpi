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
        let fs = InMemoryFs::with_home(&workspace_root);
        let session = BashSession::with_workspace(Box::new(fs), workspace_root.clone());
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

        let mut content = String::new();
        if !output.stdout.is_empty() {
            content.push_str(&output.stdout);
        }
        if !output.stderr.is_empty() {
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&output.stderr);
        }
        if output.exit_code != 0 && content.is_empty() {
            content = format!("Command exited with code {}", output.exit_code);
        }

        if output.exit_code != 0 {
            ToolResult::CommandError {
                content,
                exit_code: output.exit_code,
            }
        } else {
            ToolResult::Success { content }
        }
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

    #[tokio::test]
    async fn test_bash_nonzero_exit_returns_command_error() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        // `cat nonexistent` fails with exit code 1 in the virtual shell
        let params = serde_json::json!({
            "command": "cat nonexistent_file_for_testing"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        match result {
            ToolResult::CommandError { content, exit_code } => {
                assert_eq!(exit_code, 1, "cat of nonexistent should exit 1");
                assert!(
                    content.contains("nonexistent_file_for_testing"),
                    "expected error message about file, got: {content}"
                );
            }
            other => panic!("expected CommandError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_nonzero_exit_with_stdout() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        let params = serde_json::json!({
            "command": "echo output_before_failure && cat nonexistent_extra_file"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        match result {
            ToolResult::CommandError { content, exit_code } => {
                assert_eq!(exit_code, 1);
                assert!(
                    content.contains("output_before_failure"),
                    "expected stdout content in CommandError, got: {content}"
                );
            }
            other => panic!("expected CommandError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_nonzero_exit_with_stderr() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        let params = serde_json::json!({
            "command": "echo error_output >&2 && cat nonexistent_stderr_file"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        match result {
            ToolResult::CommandError { content, exit_code } => {
                assert_eq!(exit_code, 1);
                assert!(
                    content.contains("error_output"),
                    "expected stderr content in CommandError, got: {content}"
                );
            }
            other => panic!("expected CommandError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_command_not_found_returns_command_error() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        let params = serde_json::json!({
            "command": "nonexistent_cmd_xyzzy"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        match result {
            ToolResult::CommandError { exit_code, .. } => {
                assert_eq!(exit_code, 127, "expected exit 127 for missing command");
            }
            other => panic!("expected CommandError for missing command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_zero_exit_remains_success() {
        let tool = BashTool::new(std::path::PathBuf::from("/tmp"));
        // `echo ok` succeeds with exit code 0 in the virtual shell
        let params = serde_json::json!({
            "command": "echo ok"
        });
        let ctx = ToolContext {
            workspace_root: std::path::PathBuf::from("/tmp"),
            signal: tokio::sync::watch::channel(false).1,
        };
        let result = tool.execute(params, &ctx).await;
        assert!(
            matches!(result, ToolResult::Success { .. }),
            "echo should return Success, got {result:?}"
        );
    }
}
