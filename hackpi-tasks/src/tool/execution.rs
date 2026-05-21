use serde_json::Value;

use hackpi_core::tools::ToolResult;

use crate::slash::format_task_detail;
use crate::task::{NewTask, TaskFilter, TaskUpdate};

use super::schemas;
use super::TaskTool;

// ── Operation implementations ───────────────────────────────────────────────

impl TaskTool {
    pub(crate) async fn op_create(&self, params: &Value) -> ToolResult {
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
        match schemas::parse_priority(params) {
            Ok(Some(priority)) => input.priority = Some(priority),
            Ok(None) => {}
            Err(e) => {
                return ToolResult::SystemError { message: e };
            }
        }
        if let Some(labels) = schemas::parse_string_array(params, "labels") {
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

    pub(crate) async fn op_list(&self, params: &Value) -> ToolResult {
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

    pub(crate) async fn op_show(&self, params: &Value) -> ToolResult {
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

    pub(crate) async fn op_update(&self, params: &Value) -> ToolResult {
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
        match schemas::parse_priority(params) {
            Ok(Some(priority)) => update.priority = Some(priority),
            Ok(None) => {}
            Err(e) => {
                return ToolResult::SystemError { message: e };
            }
        }
        if let Some(labels) = schemas::parse_string_array(params, "labels") {
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

    pub(crate) async fn op_transition(&self, params: &Value) -> ToolResult {
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

    pub(crate) async fn op_block(&self, params: &Value) -> ToolResult {
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

        // Validate that the blocker task exists
        match self.store.get(blocked_by).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return ToolResult::SystemError {
                    message: format!("Cannot block: blocker task {blocked_by} does not exist."),
                }
            }
            Err(e) => {
                return ToolResult::SystemError {
                    message: format!("Failed to check blocker task {blocked_by}: {e}"),
                }
            }
        }

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

    pub(crate) async fn op_unblock(&self, params: &Value) -> ToolResult {
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
