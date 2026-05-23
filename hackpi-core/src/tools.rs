use crate::types::ToolSchema;
use async_trait::async_trait;
use hackpi_guardrails::{GuardEvaluator, GuardReason, GuardResult, PermissionDecision, ProfileToolAccess};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Construct a permission string (Claude Code format `ToolName(pattern)`)
/// from the tool name and its parameters.
///
/// Examples:
/// - `Bash(rm -rf /)` for a bash tool with a command param
/// - `Read(.env)` for a read tool with a path param
fn permission_string(tool_name: &str, params: &Value) -> String {
    if let Some(command) = params.get("command").and_then(|v| v.as_str()) {
        format!("{}({})", tool_name, command)
    } else if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
        format!("{}({})", tool_name, path)
    } else {
        format!("{}()", tool_name)
    }
}

static NEXT_PERMISSION_ID: AtomicU64 = AtomicU64::new(1);

/// A permission prompt request sent from `ToolRegistry::dispatch()` to the
/// main event loop. The receiver should use the `response` sender to convey
/// the user's decision.
pub type PermissionRequest = (u64, GuardReason, oneshot::Sender<PermissionDecision>);

pub struct ToolContext {
    pub workspace_root: std::path::PathBuf,
    pub signal: tokio::sync::watch::Receiver<bool>,
}

#[derive(Debug, Clone)]
pub enum ToolResult {
    Success {
        content: String,
    },
    SystemError {
        message: String,
    },
    /// The tool completed but returned a nonzero exit code. The `content`
    /// carries any stdout/stderr output and `exit_code` is the process exit
    /// status (e.g. 127 for command-not-found, 1 for general failure).
    CommandError {
        content: String,
        exit_code: i32,
    },
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
    permission_timeout: Duration,
    /// Active agent profile name for guard evaluation (interior mutability
    /// so callers sharing an `Arc<ToolRegistry>` can update the profile).
    active_profile_name: RwLock<Option<String>>,
    /// Active agent profile tool access map for guard evaluation.
    active_profile_access: RwLock<Option<HashMap<String, ProfileToolAccess>>>,
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
            permission_timeout: Duration::from_secs(120),
            active_profile_name: RwLock::new(None),
            active_profile_access: RwLock::new(None),
        }
    }

    /// Override the default permission prompt timeout (used in tests).
    pub fn set_permission_timeout(&mut self, timeout: Duration) {
        self.permission_timeout = timeout;
    }

    pub fn set_guard_evaluator(&mut self, evaluator: Arc<RwLock<GuardEvaluator>>) {
        self.guard_evaluator = Some(evaluator);
    }

    pub fn set_permission_tx(&mut self, tx: mpsc::UnboundedSender<PermissionRequest>) {
        self.permission_tx = Some(tx);
    }

    /// Set the active agent profile for guard evaluation.
    ///
    /// When set, `dispatch` uses `check_tool_with_profile` instead of
    /// `check_tool`, applying profile-level Deny/Ask rules before normal
    /// guardrail checks.
    pub fn set_active_profile(
        &self,
        name: Option<String>,
        access: Option<HashMap<String, ProfileToolAccess>>,
    ) {
        *self.active_profile_name.write().unwrap() = name;
        *self.active_profile_access.write().unwrap() = access;
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
                let profile_name = self.active_profile_name.read().unwrap();
                let profile_access = self.active_profile_access.read().unwrap();
                match (
                    profile_name.as_deref(),
                    profile_access.as_ref(),
                ) {
                    (Some(profile), Some(access)) => {
                        guard.check_tool_with_profile(name, &params, Some(profile), Some(access))
                    }
                    _ => guard.check_tool(name, &params),
                }
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
                    let (resp_tx, mut resp_rx) = oneshot::channel();
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

                    // Check for existing cancellation before waiting
                    if *ctx.signal.borrow() {
                        return Some(ToolResult::Cancelled);
                    }

                    // Await user response with timeout, while also listening for
                    // cancellation. Using tokio::select! ensures that cancelling
                    // during a permission prompt wakes the wait immediately instead
                    // of blocking until the permission timeout expires.
                    let mut signal_rx = ctx.signal.clone();
                    let decision = tokio::select! {
                        biased;

                        result = &mut resp_rx => {
                            match result {
                                Ok(decision) => decision,
                                Err(_) => {
                                    // Channel closed = deny
                                    return Some(ToolResult::SystemError {
                                        message: "Permission prompt cancelled.".into(),
                                    });
                                }
                            }
                        }
                        _ = tokio::time::sleep(self.permission_timeout) => {
                            return Some(ToolResult::SystemError {
                                message: "Permission prompt timed out after 120 seconds.".into(),
                            });
                        }
                        _ = signal_rx.changed() => {
                            return Some(ToolResult::Cancelled);
                        }
                    };

                    match decision {
                        PermissionDecision::AllowOnce => { /* proceed */ }
                        PermissionDecision::AllowSession => {
                            let mut guard = evaluator.write().unwrap();
                            let session_key = guard.session_cache_key(name, &params);
                            guard.record_decision(session_key, PermissionDecision::AllowSession);
                            /* proceed */
                        }
                        PermissionDecision::Deny => {
                            let mut guard = evaluator.write().unwrap();
                            let session_key = guard.session_cache_key(name, &params);
                            guard.record_decision(session_key, PermissionDecision::Deny);
                            return Some(ToolResult::SystemError {
                                message: "Permission denied by user.".into(),
                            });
                        }
                        PermissionDecision::AlwaysAllow => {
                            let perm_string = permission_string(name, &params);
                            let guard = evaluator.read().unwrap();
                            if let Err(e) = guard
                                .persist_decision(&PermissionDecision::AlwaysAllow, &perm_string)
                            {
                                // Log the error but allow execution to proceed
                                eprintln!("Warning: failed to persist AlwaysAllow: {e}");
                            }
                            /* proceed */
                        }
                        PermissionDecision::AlwaysDeny => {
                            let perm_string = permission_string(name, &params);
                            let guard = evaluator.read().unwrap();
                            if let Err(e) = guard
                                .persist_decision(&PermissionDecision::AlwaysDeny, &perm_string)
                            {
                                // Log the error but still deny execution
                                eprintln!("Warning: failed to persist AlwaysDeny: {e}");
                            }
                            return Some(ToolResult::SystemError {
                                message: "Permission denied by user. Always deny saved.".into(),
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
        let (tx, rx) = tokio::sync::watch::channel(false);
        // Keep the sender alive so signal_rx.changed() only fires on
        // actual cancellation (not when the sender is dropped).
        // In production the sender lives for the entire agent session.
        std::mem::forget(tx);
        ToolContext {
            workspace_root: std::env::temp_dir(),
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

    #[tokio::test]
    async fn test_dispatch_permission_ask_timeout_returns_system_error() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a .hackpi/guardrails.json that asks for "echo" commands
        let hackpi_dir = dir.path().join(".hackpi");
        std::fs::create_dir_all(&hackpi_dir).expect("create .hackpi dir");
        let guardrails_config = json!({
            "command_gate": {
                "ask": ["echo"]
            }
        });
        std::fs::write(
            hackpi_dir.join("guardrails.json"),
            serde_json::to_string_pretty(&guardrails_config).unwrap(),
        )
        .expect("write guardrails config");

        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);
        evaluator.load_rules().expect("load rules");
        let evaluator = Arc::new(RwLock::new(evaluator));

        // Set up permission channel — keep rx alive but never respond
        let (perm_tx, _perm_rx): (mpsc::UnboundedSender<PermissionRequest>, _) =
            mpsc::unbounded_channel();

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);
        registry.set_permission_tx(perm_tx);
        registry.set_permission_timeout(Duration::from_millis(50));

        // The "echo" command should trigger Ask, which will time out
        let result = registry
            .dispatch("echo", json!({"command": "echo test"}), &test_ctx())
            .await;

        match result {
            Some(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("timed out"),
                    "Expected timeout message, got: {message}"
                );
            }
            other => {
                panic!("Expected SystemError with timeout, got: {other:?}");
            }
        }
    }

    #[tokio::test]
    async fn test_dispatch_permission_ask_allow_once_executes() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a .hackpi/guardrails.json that asks for "echo" commands
        let hackpi_dir = dir.path().join(".hackpi");
        std::fs::create_dir_all(&hackpi_dir).expect("create .hackpi dir");
        let guardrails_config = json!({
            "command_gate": {
                "ask": ["echo"]
            }
        });
        std::fs::write(
            hackpi_dir.join("guardrails.json"),
            serde_json::to_string_pretty(&guardrails_config).unwrap(),
        )
        .expect("write guardrails config");

        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);
        evaluator.load_rules().expect("load rules");
        let evaluator = Arc::new(RwLock::new(evaluator));

        // Set up permission channel and respond with AllowOnce
        let (perm_tx, mut perm_rx): (mpsc::UnboundedSender<PermissionRequest>, _) =
            mpsc::unbounded_channel();

        // Spawn a task that receives the permission request and responds
        tokio::spawn(async move {
            if let Some((_id, _reason, resp_tx)) = perm_rx.recv().await {
                let _ = resp_tx.send(PermissionDecision::AllowOnce);
            }
        });

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);
        registry.set_permission_tx(perm_tx);
        registry.set_permission_timeout(Duration::from_secs(120));

        let result = registry
            .dispatch("echo", json!({"command": "echo test"}), &test_ctx())
            .await;

        assert!(
            matches!(result, Some(ToolResult::Success { .. })),
            "Expected Success after AllowOnce, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_permission_ask_cancelled_before_wait() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a .hackpi/guardrails.json that asks for "echo" commands
        let hackpi_dir = dir.path().join(".hackpi");
        std::fs::create_dir_all(&hackpi_dir).expect("create .hackpi dir");
        let guardrails_config = json!({
            "command_gate": {
                "ask": ["echo"]
            }
        });
        std::fs::write(
            hackpi_dir.join("guardrails.json"),
            serde_json::to_string_pretty(&guardrails_config).unwrap(),
        )
        .expect("write guardrails config");

        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);
        evaluator.load_rules().expect("load rules");
        let evaluator = Arc::new(RwLock::new(evaluator));

        let (perm_tx, _perm_rx): (mpsc::UnboundedSender<PermissionRequest>, _) =
            mpsc::unbounded_channel();

        // Create a context where cancellation is already signalled
        let (_signal_tx, signal_rx) = tokio::sync::watch::channel(true);
        let ctx = ToolContext {
            workspace_root: std::env::temp_dir(),
            signal: signal_rx,
        };

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);
        registry.set_permission_tx(perm_tx);
        registry.set_permission_timeout(Duration::from_secs(120));

        // Signal is already true → should return Cancelled immediately
        let result = registry
            .dispatch("echo", json!({"command": "echo test"}), &ctx)
            .await;

        assert!(
            matches!(result, Some(ToolResult::Cancelled)),
            "Expected Cancelled when signal pre-set, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_permission_ask_cancelled_during_wait() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create a .hackpi/guardrails.json that asks for "echo" commands
        let hackpi_dir = dir.path().join(".hackpi");
        std::fs::create_dir_all(&hackpi_dir).expect("create .hackpi dir");
        let guardrails_config = json!({
            "command_gate": {
                "ask": ["echo"]
            }
        });
        std::fs::write(
            hackpi_dir.join("guardrails.json"),
            serde_json::to_string_pretty(&guardrails_config).unwrap(),
        )
        .expect("write guardrails config");

        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);
        evaluator.load_rules().expect("load rules");
        let evaluator = Arc::new(RwLock::new(evaluator));

        let (perm_tx, _perm_rx): (mpsc::UnboundedSender<PermissionRequest>, _) =
            mpsc::unbounded_channel();

        // Create a cancellable signal that starts as false
        let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);
        let ctx = ToolContext {
            workspace_root: std::env::temp_dir(),
            signal: signal_rx,
        };

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);
        registry.set_permission_tx(perm_tx);
        registry.set_permission_timeout(Duration::from_secs(120));

        // Spawn dispatch — it will enter the permission prompt wait
        let handle = tokio::spawn(async move {
            registry
                .dispatch("echo", json!({"command": "echo test"}), &ctx)
                .await
        });

        // Give it a moment to enter the permission wait
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send cancellation signal — should wake the wait
        signal_tx.send(true).expect("send cancellation");

        // The dispatch should now return Cancelled
        let result = handle.await.expect("dispatch task panicked");
        assert!(
            matches!(result, Some(ToolResult::Cancelled)),
            "Expected Cancelled after mid-wait cancellation, got: {result:?}"
        );
    }

    // ── set_active_profile + dispatch integration tests ─────────────────

    #[tokio::test]
    async fn test_dispatch_with_profile_deny_blocks_tool() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(false, paths)));

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        // Set profile that denies "bash"
        let mut access = HashMap::new();
        access.insert("bash".to_string(), ProfileToolAccess::Deny);
        registry.set_active_profile(Some("strict".to_string()), Some(access));

        let result = registry
            .dispatch("bash", json!({"command": "rm -rf /"}), &test_ctx())
            .await;

        match result {
            Some(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("denied by agent profile"),
                    "Expected profile deny message, got: {message}"
                );
            }
            other => panic!("Expected SystemError from profile deny, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dispatch_with_profile_ask_prompts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(false, paths)));

        let (perm_tx, mut perm_rx): (mpsc::UnboundedSender<PermissionRequest>, _) =
            mpsc::unbounded_channel();

        // Spawn a task that receives the permission request and responds AllowOnce
        tokio::spawn(async move {
            if let Some((_id, _reason, resp_tx)) = perm_rx.recv().await {
                let _ = resp_tx.send(PermissionDecision::AllowOnce);
            }
        });

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);
        registry.set_permission_tx(perm_tx);

        // Set profile that asks for "echo"
        let mut access = HashMap::new();
        access.insert("echo".to_string(), ProfileToolAccess::Ask);
        registry.set_active_profile(Some("cautious".to_string()), Some(access));

        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;

        assert!(
            matches!(result, Some(ToolResult::Success { .. })),
            "Expected Success after AllowOnce for profile Ask, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_with_profile_allow_passes_through() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(false, paths)));

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        // Set profile that allows "echo" — no guardrail rules loaded → Allow
        let mut access = HashMap::new();
        access.insert("echo".to_string(), ProfileToolAccess::Allow);
        registry.set_active_profile(Some("permissive".to_string()), Some(access));

        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;

        assert!(
            matches!(result, Some(ToolResult::Success { .. })),
            "Expected Success with profile Allow, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_clears_profile_when_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = Arc::new(RwLock::new(GuardEvaluator::new(false, paths)));

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        // Set profile that denies "echo"
        let mut access = HashMap::new();
        access.insert("echo".to_string(), ProfileToolAccess::Deny);
        registry.set_active_profile(Some("strict".to_string()), Some(access));

        // Verify deny is active
        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(
            matches!(result, Some(ToolResult::SystemError { .. })),
            "Expected deny before clear, got: {result:?}"
        );

        // Clear the profile
        registry.set_active_profile(None, None);

        // Now dispatch should succeed — no profile rules, no guardrails
        let result = registry.dispatch("echo", json!({}), &test_ctx()).await;
        assert!(
            matches!(result, Some(ToolResult::Success { .. })),
            "Expected Success after clearing profile, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_dispatch_profile_deny_overrides_guardrail_allow() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Create guardrails config that allows bash via permissions.allow
        let hackpi_dir = dir.path().join(".hackpi");
        std::fs::create_dir_all(&hackpi_dir).expect("create .hackpi dir");
        let guardrails_config = json!({
            "permissions": {
                "allow": ["Bash(echo hello)"]
            }
        });
        std::fs::write(
            hackpi_dir.join("guardrails.json"),
            serde_json::to_string_pretty(&guardrails_config).unwrap(),
        )
        .expect("write guardrails config");

        let paths = SettingsPaths::new(dir.path());
        let mut evaluator = GuardEvaluator::new(false, paths);
        evaluator.load_rules().expect("load rules");
        let evaluator = Arc::new(RwLock::new(evaluator));

        let mut registry = make_registry();
        registry.set_guard_evaluator(evaluator);

        // Set profile that denies "bash" — overrides guardrail allow
        let mut access = HashMap::new();
        access.insert("bash".to_string(), ProfileToolAccess::Deny);
        registry.set_active_profile(Some("strict".to_string()), Some(access));

        let result = registry
            .dispatch("bash", json!({"command": "echo hello"}), &test_ctx())
            .await;

        match result {
            Some(ToolResult::SystemError { message }) => {
                assert!(
                    message.contains("denied by agent profile"),
                    "Expected profile deny to override guardrail allow, got: {message}"
                );
            }
            other => panic!(
                "Expected SystemError from profile deny overriding guardrail, got: {other:?}"
            ),
        }
    }
}
