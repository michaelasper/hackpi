use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::path::PathBuf;

use super::session::with_session;

pub struct BashTool {
    workspace_root: PathBuf,
}

impl BashTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
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
            "required": ["command"]
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

        let wr = self.workspace_root.clone();
        let signal = ctx.signal.clone();

        let timeout_dur = std::time::Duration::from_secs(timeout_secs);

        let result = tokio::time::timeout(timeout_dur, async {
            tokio::task::block_in_place(|| {
                with_session(&wr, workdir, Some(signal), |session| {
                    session.execute(command)
                })
            })
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
