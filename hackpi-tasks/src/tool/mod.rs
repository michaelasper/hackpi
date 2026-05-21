pub(crate) mod execution;
pub mod formatting;
pub(crate) mod registry;
pub mod schemas;

use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::Value;
use std::sync::Arc;

use crate::store::TaskStore;

/// Tool that exposes task management operations to the LLM agent.
///
/// Operations mirror the slash commands but are invoked as a tool call
/// from the agent loop. The tool holds a shared reference to the same
/// `TaskStore` used by slash commands.
pub struct TaskTool {
    pub(crate) store: Arc<dyn TaskStore>,
}

impl TaskTool {
    pub fn new(store: Arc<dyn TaskStore>) -> Self {
        Self { store }
    }

    /// Register this tool with a `ToolRegistry`.
    pub fn register(self, registry: &mut hackpi_core::tools::ToolRegistry) {
        registry.register(Box::new(self));
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Manage tasks: create, list, show, update, transition, block, unblock. \
         Tasks track work items with workflow-defined states."
    }

    fn input_schema(&self) -> Value {
        schemas::build_input_schema()
    }

    async fn execute(&self, params: Value, _ctx: &ToolContext) -> ToolResult {
        let operation = match params.get("operation").and_then(|v| v.as_str()) {
            Some(op) => op,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'operation' parameter. \
                              Available operations: create, list, show, update, transition, block, unblock"
                        .into(),
                }
            }
        };

        match operation {
            "create" => self.op_create(&params).await,
            "list" => self.op_list(&params).await,
            "show" => self.op_show(&params).await,
            "update" => self.op_update(&params).await,
            "transition" => self.op_transition(&params).await,
            "block" => self.op_block(&params).await,
            "unblock" => self.op_unblock(&params).await,
            _ => ToolResult::SystemError {
                message: format!(
                    "Unknown operation: '{operation}'. \
                     Available operations: create, list, show, update, transition, block, unblock"
                ),
            },
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{NewTask, TaskPriority, TaskUpdate};
    use hackpi_core::tools::ToolContext;
    use serde_json::json;

    /// Helper: create a fresh JsonTaskStore in a temp directory.
    async fn setup_store() -> (tempfile::TempDir, Arc<dyn TaskStore>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let store = Arc::new(
            crate::store::JsonTaskStore::new(tasks_dir)
                .await
                .expect("create store"),
        );
        (dir, store)
    }

    fn make_tool(store: Arc<dyn TaskStore>) -> TaskTool {
        TaskTool::new(store)
    }

    fn test_ctx() -> ToolContext {
        ToolContext {
            workspace_root: std::env::temp_dir(),
            signal: tokio::sync::watch::channel(false).1,
        }
    }

    async fn execute(tool: &TaskTool, params: Value) -> ToolResult {
        tool.execute(params, &test_ctx()).await
    }

    // ── Basic tool metadata ──────────────────────────────────────────────

    #[test]
    fn test_name() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, store) = rt.block_on(setup_store());
        let tool = make_tool(store);
        assert_eq!(tool.name(), "task");
    }

    #[test]
    fn test_description() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, store) = rt.block_on(setup_store());
        let tool = make_tool(store);
        assert!(tool.description().contains("create"));
        assert!(tool.description().contains("list"));
        assert!(tool.description().contains("transition"));
    }

    #[test]
    fn test_input_schema_has_additional_properties_false() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, store) = rt.block_on(setup_store());
        let tool = make_tool(store);
        let schema = tool.input_schema();
        assert_eq!(
            schema.get("additionalProperties"),
            Some(&json!(false)),
            "task tool schema missing additionalProperties: false"
        );
        assert!(schema.get("properties").unwrap().get("operation").is_some());
    }

    #[test]
    fn test_input_schema_operation_enum() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, store) = rt.block_on(setup_store());
        let tool = make_tool(store);
        let schema = tool.input_schema();
        let ops = &schema["properties"]["operation"]["enum"];
        let expected = json!([
            "create",
            "list",
            "show",
            "update",
            "transition",
            "block",
            "unblock"
        ]);
        assert_eq!(ops, &expected);
    }

    #[test]
    fn test_input_schema_required_fields() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (_dir, store) = rt.block_on(setup_store());
        let tool = make_tool(store);
        let schema = tool.input_schema();
        assert_eq!(schema["required"], json!(["operation"]));
    }

    // ── Missing operation ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_missing_operation_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({})).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'operation'")),
            "Expected SystemError for missing operation, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_unknown_operation_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "delete" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Unknown operation")),
            "Expected SystemError for unknown operation, got: {result:?}"
        );
    }

    // ── Create operation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_basic_task() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "create", "title": "Add logging" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(
                    content.contains("Created TSK-001"),
                    "Expected 'Created TSK-001', got: {content}"
                );
                assert!(
                    content.contains("Add logging"),
                    "Expected title in output, got: {content}"
                );
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_missing_title_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "create" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'title'")),
            "Expected SystemError for missing title, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_create_with_all_fields() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store.clone());
        let result = execute(
            &tool,
            json!({
                "operation": "create",
                "title": "Complex task",
                "description": "Detailed description",
                "priority": "high",
                "labels": ["backend", "rust"],
                "assignee": "alice"
            }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Created TSK-001"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Verify the task was created with correct fields
        let task = store.get("TSK-001").await.unwrap().unwrap();
        assert_eq!(task.title, "Complex task");
        assert_eq!(task.description, "Detailed description");
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.labels, vec!["backend", "rust"]);
        assert_eq!(task.assignee, Some("alice".to_string()));
    }

    #[tokio::test]
    async fn test_create_with_invalid_priority_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({
                "operation": "create",
                "title": "Task",
                "priority": "super_high"
            }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Invalid priority")),
            "Expected SystemError for invalid priority, got: {result:?}"
        );
    }

    // ── List operation ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_list_empty() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "No tasks found.");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_list_with_tasks() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task 1")).await.unwrap();
        store.create(&NewTask::new("Task 2")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("TSK-001 [todo] Task 1"));
                assert!(content.contains("TSK-002 [todo] Task 2"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_list_with_filter() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Todo task")).await.unwrap();

        let in_progress = store
            .create(&NewTask::new("In progress task"))
            .await
            .unwrap();
        store
            .update(
                &in_progress.id,
                &TaskUpdate {
                    state: Some("in_progress".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "list", "filter_state": "in_progress" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("TSK-002"));
                assert!(content.contains("in_progress"));
                assert!(!content.contains("TSK-001"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Show operation ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_show_existing_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("My task")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "show", "id": "TSK-001" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("TSK-001"));
                assert!(content.contains("My task"));
                assert!(content.contains("todo"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_show_missing_id_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "show" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'id'")),
            "Expected SystemError for missing id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_show_not_found() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "show", "id": "TSK-999" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for not found, got: {result:?}"
        );
    }

    // ── Update operation ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_update_title() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Original")).await.unwrap();

        let tool = make_tool(store.clone());
        let result = execute(
            &tool,
            json!({ "operation": "update", "id": "TSK-001", "title": "Updated title" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Updated TSK-001"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        let task = store.get("TSK-001").await.unwrap().unwrap();
        assert_eq!(task.title, "Updated title");
    }

    #[tokio::test]
    async fn test_update_missing_id_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "update" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'id'")),
            "Expected SystemError for missing id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_update_not_found() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "update", "id": "TSK-999", "title": "Nope" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for not found, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_update_multiple_fields() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let tool = make_tool(store.clone());
        let result = execute(
            &tool,
            json!({
                "operation": "update",
                "id": "TSK-001",
                "title": "Updated",
                "description": "New description",
                "priority": "urgent",
                "labels": ["critical"],
                "assignee": "bob"
            }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Updated TSK-001"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        let task = store.get("TSK-001").await.unwrap().unwrap();
        assert_eq!(task.title, "Updated");
        assert_eq!(task.description, "New description");
        assert_eq!(task.priority, TaskPriority::Urgent);
        assert_eq!(task.labels, vec!["critical"]);
        assert_eq!(task.assignee, Some("bob".to_string()));
    }

    // ── Transition operation ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_transition_valid() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-001", "state": "in_progress" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Transitioned TSK-001"));
                assert!(content.contains("todo → in_progress"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_transition_missing_id_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "transition", "state": "in_progress" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'id'")),
            "Expected SystemError for missing id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_transition_missing_state_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "transition", "id": "TSK-001" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'state'")),
            "Expected SystemError for missing state, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_transition_invalid_transition() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-001", "state": "done" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Invalid transition")),
            "Expected SystemError for invalid transition, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_transition_not_found() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-999", "state": "in_progress" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for not found, got: {result:?}"
        );
    }

    // ── Block operation ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_block_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-002", "blocked_by": "TSK-001" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "TSK-002 now blocked by TSK-001");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_block_already_blocked() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        let tool = make_tool(store);
        execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-002", "blocked_by": "TSK-001" }),
        )
        .await;

        let result = execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-002", "blocked_by": "TSK-001" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "TSK-002 is already blocked by TSK-001");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_block_missing_id_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "block", "blocked_by": "TSK-001" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'id'")),
            "Expected SystemError for missing id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_block_missing_blocked_by_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "block", "id": "TSK-002" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'blocked_by'")),
            "Expected SystemError for missing blocked_by, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_block_not_found() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-999", "blocked_by": "TSK-001" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("not found")),
            "Expected SystemError for not found, got: {result:?}"
        );
    }

    // ── Unblock operation ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_unblock_task() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Blocker")).await.unwrap();
        store.create(&NewTask::new("Blocked")).await.unwrap();

        let tool = make_tool(store);
        execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-002", "blocked_by": "TSK-001" }),
        )
        .await;

        let result = execute(
            &tool,
            json!({ "operation": "unblock", "id": "TSK-002", "blocked_by": "TSK-001" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "Removed TSK-001 from TSK-002 blockers");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_unblock_not_blocked() {
        let (_dir, store) = setup_store().await;
        store.create(&NewTask::new("Task")).await.unwrap();

        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "unblock", "id": "TSK-001", "blocked_by": "TSK-002" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "TSK-001 is not blocked by TSK-002");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_unblock_missing_id_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(
            &tool,
            json!({ "operation": "unblock", "blocked_by": "TSK-001" }),
        )
        .await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'id'")),
            "Expected SystemError for missing id, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_unblock_missing_blocked_by_returns_error() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store);
        let result = execute(&tool, json!({ "operation": "unblock", "id": "TSK-001" })).await;
        assert!(
            matches!(&result, ToolResult::SystemError { message } if message.contains("Missing 'blocked_by'")),
            "Expected SystemError for missing blocked_by, got: {result:?}"
        );
    }

    // ── Integration: Full lifecycle ──────────────────────────────────────

    #[tokio::test]
    async fn test_full_lifecycle_create_transition_block_complete() {
        let (_dir, store) = setup_store().await;
        let tool = make_tool(store.clone());

        // Create
        let result = execute(
            &tool,
            json!({ "operation": "create", "title": "Implement auth", "priority": "high" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Created TSK-001"));
                assert!(content.contains("Implement auth"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Create a blocker task
        let result = execute(
            &tool,
            json!({ "operation": "create", "title": "Setup database" }),
        )
        .await;
        assert!(matches!(result, ToolResult::Success { .. }));

        // List
        let result = execute(&tool, json!({ "operation": "list" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("TSK-001"));
                assert!(content.contains("TSK-002"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Block
        let result = execute(
            &tool,
            json!({ "operation": "block", "id": "TSK-001", "blocked_by": "TSK-002" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "TSK-001 now blocked by TSK-002");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Unblock
        let result = execute(
            &tool,
            json!({ "operation": "unblock", "id": "TSK-001", "blocked_by": "TSK-002" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert_eq!(content, "Removed TSK-002 from TSK-001 blockers");
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Show
        let result = execute(&tool, json!({ "operation": "show", "id": "TSK-001" })).await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("Implement auth"));
                assert!(content.contains("todo"));
                assert!(content.contains("High"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Transition: todo → in_progress
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-001", "state": "in_progress" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("todo → in_progress"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }

        // Transition: in_progress → in_review
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-001", "state": "in_review" }),
        )
        .await;
        assert!(matches!(result, ToolResult::Success { .. }));

        // Transition: in_review → done
        let result = execute(
            &tool,
            json!({ "operation": "transition", "id": "TSK-001", "state": "done" }),
        )
        .await;
        match &result {
            ToolResult::Success { content } => {
                assert!(content.contains("in_review → done"));
            }
            _ => panic!("Expected Success, got: {result:?}"),
        }
    }

    // ── Registration test ────────────────────────────────────────────────

    #[test]
    fn test_register_with_tool_registry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let store: Arc<dyn TaskStore> = Arc::new(
            rt.block_on(crate::store::JsonTaskStore::new(tasks_dir))
                .unwrap(),
        );

        let mut registry = hackpi_core::tools::ToolRegistry::new();
        let tool = TaskTool::new(store);
        tool.register(&mut registry);

        assert!(
            registry.get("task").is_some(),
            "task tool should be registered"
        );
    }
}
