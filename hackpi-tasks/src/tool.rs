use async_trait::async_trait;
use hackpi_core::tools::{Tool, ToolContext, ToolResult};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::slash::format_task_detail;
use crate::store::TaskStore;
use crate::task::{NewTask, TaskFilter, TaskPriority, TaskUpdate};

/// Tool that exposes task management operations to the LLM agent.
///
/// Operations mirror the slash commands but are invoked as a tool call
/// from the agent loop. The tool holds a shared reference to the same
/// `TaskStore` used by slash commands.
pub struct TaskTool {
    store: Arc<dyn TaskStore>,
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
         Tasks track work items with states (todo, in_progress, in_review, done, cancelled)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "list", "show", "update", "transition", "block", "unblock"],
                    "description": "The task operation to perform"
                },
                "title": {
                    "type": "string",
                    "description": "Task title (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (optional for create, update)"
                },
                "id": {
                    "type": "string",
                    "description": "Task ID in TSK-XXX format (required for show, update, transition, block, unblock)"
                },
                "state": {
                    "type": "string",
                    "description": "Target state for transition (e.g., in_progress, in_review, done, cancelled)"
                },
                "priority": {
                    "type": "string",
                    "enum": ["none", "low", "medium", "high", "urgent"],
                    "description": "Priority level (for create or update)"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels for the task (for create or update)"
                },
                "assignee": {
                    "type": "string",
                    "description": "Assignee identifier (for create or update)"
                },
                "blocked_by": {
                    "type": "string",
                    "description": "ID of the blocking task (for block/unblock)"
                },
                "filter_state": {
                    "type": "string",
                    "description": "Filter tasks by state (for list)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
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

// ── Operation implementations ───────────────────────────────────────────────

impl TaskTool {
    async fn op_create(&self, params: &Value) -> ToolResult {
        let title = match params.get("title").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'title' parameter for create. \
                              Usage: { \"operation\": \"create\", \"title\": \"Task title\" }"
                        .into(),
                }
            }
        };

        let mut input = NewTask::new(title);

        if let Some(desc) = params.get("description").and_then(|v| v.as_str()) {
            input.description = Some(desc.to_string());
        }
        match parse_priority(params) {
            Ok(Some(priority)) => input.priority = Some(priority),
            Ok(None) => {}
            Err(e) => {
                return ToolResult::SystemError { message: e };
            }
        }
        if let Some(labels) = parse_string_array(params, "labels") {
            input.labels = Some(labels);
        }
        if let Some(assignee) = params.get("assignee").and_then(|v| v.as_str()) {
            input.assignee = Some(assignee.to_string());
        }

        match self.store.create(&input).await {
            Ok(task) => ToolResult::Success {
                content: format!("Created {}: \"{}\"", task.id, task.title),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to create task: {e}"),
            },
        }
    }

    async fn op_list(&self, params: &Value) -> ToolResult {
        let mut filter = TaskFilter::default();
        if let Some(state) = params.get("filter_state").and_then(|v| v.as_str()) {
            filter.state = Some(state.to_string());
        }

        match self.store.list(&filter).await {
            Ok(tasks) => {
                if tasks.is_empty() {
                    return ToolResult::Success {
                        content: "No tasks found.".to_string(),
                    };
                }
                let mut lines = Vec::with_capacity(tasks.len());
                for task in &tasks {
                    lines.push(format!("{} [{}] {}", task.id, task.state, task.title));
                }
                ToolResult::Success {
                    content: lines.join("\n"),
                }
            }
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to list tasks: {e}"),
            },
        }
    }

    async fn op_show(&self, params: &Value) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'id' parameter for show. \
                              Usage: { \"operation\": \"show\", \"id\": \"TSK-001\" }"
                        .into(),
                }
            }
        };

        match self.store.get(id).await {
            Ok(Some(task)) => ToolResult::Success {
                content: format_task_detail(&task),
            },
            Ok(None) => ToolResult::SystemError {
                message: format!("Task {id} not found."),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to get task {id}: {e}"),
            },
        }
    }

    async fn op_update(&self, params: &Value) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'id' parameter for update. \
                              Usage: { \"operation\": \"update\", \"id\": \"TSK-001\", ... }"
                        .into(),
                }
            }
        };

        // Check the task exists first
        match self.store.get(id).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return ToolResult::SystemError {
                    message: format!("Task {id} not found."),
                }
            }
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get task {id}: {e}"),
                }
            }
        }

        let mut update = TaskUpdate::default();
        if let Some(title) = params.get("title").and_then(|v| v.as_str()) {
            update.title = Some(title.to_string());
        }
        if let Some(description) = params.get("description").and_then(|v| v.as_str()) {
            update.description = Some(description.to_string());
        }
        match parse_priority(params) {
            Ok(Some(priority)) => update.priority = Some(priority),
            Ok(None) => {}
            Err(e) => {
                return ToolResult::SystemError { message: e };
            }
        }
        if let Some(labels) = parse_string_array(params, "labels") {
            update.labels = Some(labels);
        }
        if let Some(assignee) = params.get("assignee").and_then(|v| v.as_str()) {
            update.assignee = Some(Some(assignee.to_string()));
        }

        match self.store.update(id, &update).await {
            Ok(Some(_)) => ToolResult::Success {
                content: format!("Updated {id}"),
            },
            Ok(None) => ToolResult::SystemError {
                message: format!("Task {id} not found during update."),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to update task {id}: {e}"),
            },
        }
    }

    async fn op_transition(&self, params: &Value) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'id' parameter for transition. \
                              Usage: { \"operation\": \"transition\", \"id\": \"TSK-001\", \"state\": \"in_progress\" }"
                        .into(),
                }
            }
        };

        let state = match params.get("state").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'state' parameter for transition. \
                              Usage: { \"operation\": \"transition\", \"id\": \"TSK-001\", \"state\": \"in_progress\" }"
                        .into(),
                }
            }
        };

        let existing = match self.store.get(id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                return ToolResult::SystemError {
                    message: format!("Task {id} not found."),
                }
            }
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get task {id}: {e}"),
                }
            }
        };

        let old_state = existing.state.clone();
        let update = TaskUpdate {
            state: Some(state.to_string()),
            ..Default::default()
        };

        match self.store.update(id, &update).await {
            Ok(Some(_)) => ToolResult::Success {
                content: format!("Transitioned {id}: {old_state} → {state}"),
            },
            Ok(None) => ToolResult::SystemError {
                message: format!("Task {id} not found during transition."),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to transition task {id}: {e}"),
            },
        }
    }

    async fn op_block(&self, params: &Value) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'id' parameter for block. \
                              Usage: { \"operation\": \"block\", \"id\": \"TSK-003\", \"blocked_by\": \"TSK-001\" }"
                        .into(),
                }
            }
        };

        let blocked_by = match params.get("blocked_by").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'blocked_by' parameter for block. \
                              Usage: { \"operation\": \"block\", \"id\": \"TSK-003\", \"blocked_by\": \"TSK-001\" }"
                        .into(),
                }
            }
        };

        let existing = match self.store.get(id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                return ToolResult::SystemError {
                    message: format!("Task {id} not found."),
                }
            }
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get task {id}: {e}"),
                }
            }
        };

        let mut blockers = existing.blocked_by.clone();
        if blockers.contains(&blocked_by.to_string()) {
            return ToolResult::Success {
                content: format!("{id} is already blocked by {blocked_by}"),
            };
        }
        blockers.push(blocked_by.to_string());

        let update = TaskUpdate {
            blocked_by: Some(blockers),
            ..Default::default()
        };

        match self.store.update(id, &update).await {
            Ok(Some(_)) => ToolResult::Success {
                content: format!("{id} now blocked by {blocked_by}"),
            },
            Ok(None) => ToolResult::SystemError {
                message: format!("Task {id} not found during block."),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to block task {id}: {e}"),
            },
        }
    }

    async fn op_unblock(&self, params: &Value) -> ToolResult {
        let id = match params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'id' parameter for unblock. \
                              Usage: { \"operation\": \"unblock\", \"id\": \"TSK-003\", \"blocked_by\": \"TSK-001\" }"
                        .into(),
                }
            }
        };

        let blocked_by = match params.get("blocked_by").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => {
                return ToolResult::SystemError {
                    message: "Missing 'blocked_by' parameter for unblock. \
                              Usage: { \"operation\": \"unblock\", \"id\": \"TSK-003\", \"blocked_by\": \"TSK-001\" }"
                        .into(),
                }
            }
        };

        let existing = match self.store.get(id).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                return ToolResult::SystemError {
                    message: format!("Task {id} not found."),
                }
            }
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to get task {id}: {e}"),
                }
            }
        };

        let mut blockers = existing.blocked_by.clone();
        if !blockers.contains(&blocked_by.to_string()) {
            return ToolResult::Success {
                content: format!("{id} is not blocked by {blocked_by}"),
            };
        }
        blockers.retain(|b| b != blocked_by);

        let update = TaskUpdate {
            blocked_by: Some(blockers),
            ..Default::default()
        };

        match self.store.update(id, &update).await {
            Ok(Some(_)) => ToolResult::Success {
                content: format!("Removed {blocked_by} from {id} blockers"),
            },
            Ok(None) => ToolResult::SystemError {
                message: format!("Task {id} not found during unblock."),
            },
            Err(e) => ToolResult::SystemError {
                message: format!("Failed to unblock task {id}: {e}"),
            },
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn parse_priority(params: &Value) -> Result<Option<TaskPriority>, String> {
    match params.get("priority").and_then(|v| v.as_str()) {
        Some(s) => match s {
            "none" => Ok(Some(TaskPriority::None)),
            "low" => Ok(Some(TaskPriority::Low)),
            "medium" => Ok(Some(TaskPriority::Medium)),
            "high" => Ok(Some(TaskPriority::High)),
            "urgent" => Ok(Some(TaskPriority::Urgent)),
            _ => Err(format!(
                "Invalid priority: '{s}'. Valid values: none, low, medium, high, urgent"
            )),
        },
        None => Ok(None),
    }
}

fn parse_string_array(params: &Value, key: &str) -> Option<Vec<String>> {
    params.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_core::tools::ToolContext;

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
