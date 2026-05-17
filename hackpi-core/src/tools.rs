use crate::types::ToolSchema;
use async_trait::async_trait;
use hackpi_guardrails::{GuardEvaluator, GuardReason, GuardResult, PermissionDecision};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{mpsc, oneshot};

static NEXT_PERMISSION_ID: AtomicU64 = AtomicU64::new(1);

/// A permission prompt request sent from `ToolRegistry::dispatch()` to the
/// main event loop. The receiver should use the `response` sender to convey
/// the user's decision.
pub type PermissionRequest = (u64, GuardReason, oneshot::Sender<PermissionDecision>);

pub struct ToolContext {
    pub workspace_root: std::path::PathBuf,
    pub conversation_id: String,
    pub signal: tokio::sync::watch::Receiver<bool>,
}

#[derive(Debug, Clone)]
pub enum ToolResult {
    Success { content: String },
    SystemError { message: String },
    Timeout,
    Cancelled,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    guard_evaluator: Option<Arc<RwLock<GuardEvaluator>>>,
    permission_tx: Option<mpsc::UnboundedSender<PermissionRequest>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            guard_evaluator: None,
            permission_tx: None,
        }
    }

    pub fn set_guard_evaluator(&mut self, evaluator: Arc<RwLock<GuardEvaluator>>) {
        self.guard_evaluator = Some(evaluator);
    }

    pub fn set_permission_tx(&mut self, tx: mpsc::UnboundedSender<PermissionRequest>) {
        self.permission_tx = Some(tx);
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn all_schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub async fn dispatch(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        // Step 1: Check with GuardEvaluator if present
        if let Some(evaluator) = &self.guard_evaluator {
            let result = {
                let guard = evaluator.read().unwrap();
                guard.check_tool(name, &params)
            }; // guard dropped here, before any await points

            match result {
                GuardResult::Allow => { /* proceed */ }
                GuardResult::Deny(reason) => {
                    return Some(ToolResult::SystemError {
                        message: format!("Blocked by guardrails: {reason}"),
                    });
                }
                GuardResult::Ask(guard_reason) => {
                    // Create oneshot channel for the user's decision
                    let (resp_tx, resp_rx) = oneshot::channel();
                    let id = NEXT_PERMISSION_ID.fetch_add(1, Ordering::Relaxed);

                    // Send PermissionRequest through the channel to the main loop
                    let reason_for_channel = hackpi_guardrails::GuardReason {
                        guard: guard_reason.guard.clone(),
                        tool: guard_reason.tool.clone(),
                        details: guard_reason.details.clone(),
                    };
                    if let Some(tx) = &self.permission_tx {
                        if tx.send((id, reason_for_channel, resp_tx)).is_err() {
                            return Some(ToolResult::SystemError {
                                message: "Permission prompt channel closed.".into(),
                            });
                        }
                    } else {
                        return Some(ToolResult::SystemError {
                            message: "Permission prompt not available.".into(),
                        });
                    }

                    // Await user response
                    match resp_rx.await {
                        Ok(PermissionDecision::AllowOnce) => { /* proceed */ }
                        Ok(PermissionDecision::AllowSession) => {
                            let mut guard = evaluator.write().unwrap();
                            let session_key =
                                format!("{}:{}", guard_reason.tool, guard_reason.details);
                            guard.record_decision(session_key, PermissionDecision::AllowSession);
                            /* proceed */
                        }
                        Ok(PermissionDecision::Deny) => {
                            return Some(ToolResult::SystemError {
                                message: "Permission denied by user.".into(),
                            });
                        }
                        Ok(PermissionDecision::AlwaysAllow) => {
                            // persist to config (future: write rule to config file)
                            /* proceed */
                        }
                        Ok(PermissionDecision::AlwaysDeny) => {
                            // persist to config (future: write rule to config file)
                            return Some(ToolResult::SystemError {
                                message: "Permission denied by user.".into(),
                            });
                        }
                        Err(_) => {
                            // Channel closed = deny
                            return Some(ToolResult::SystemError {
                                message: "Permission prompt cancelled.".into(),
                            });
                        }
                    }
                }
            }
        }

        // Step 2: Execute the tool
        let tool = self.get(name)?;
        Some(tool.execute(params, ctx).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolResult;
    use hackpi_guardrails::SettingsPaths;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::RwLock;

    struct PassthroughTool;

    #[async_trait]
    impl Tool for PassthroughTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echoes input"
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }
        async fn execute(&self, _params: Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::Success {
                content: "ok".into(),
            }
        }
    }

    fn test_ctx() -> ToolContext {
        let (_tx, rx) = tokio::sync::watch::channel(false);
        ToolContext {
            workspace_root: std::env::temp_dir(),
            conversation_id: String::new(),
            signal: rx,
        }
    }

    fn make_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(PassthroughTool));
        reg
    }

    #[tokio::test]
    async fn test_dispatch_no_guard_executes_normally() {
        let registry = make_registry();
        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(matches!(result, Some(ToolResult::Success { .. })));
    }

    #[tokio::test]
    async fn test_dispatch_with_allow_guard_executes_normally() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(false, paths)));

        // No rules loaded → everything is allowed by default
        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(matches!(result, Some(ToolResult::Success { .. })));
    }

    #[tokio::test]
    async fn test_dispatch_with_god_mode_bypasses_checks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(true, paths)));

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        // god_mode should bypass all checks
        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(matches!(result, Some(ToolResult::Success { .. })));
    }

    #[tokio::test]
    async fn test_dispatch_with_unknown_tool_returns_none() {
        let registry = make_registry();
        let result = registry
            .dispatch("nonexistent", json!({}), &test_ctx())
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_dispatch_no_registry_fallback_executes_normally() {
        let registry = make_registry();
        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(matches!(result, Some(ToolResult::Success { .. })));
    }
}
