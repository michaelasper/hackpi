use serde_json::{json, Value};

use crate::task::TaskPriority;

/// Build the JSON input schema for the task tool.
pub(crate) fn build_input_schema() -> Value {
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

/// Parse a priority string from tool parameters.
pub(crate) fn parse_priority(params: &Value) -> Result<Option<TaskPriority>, String> {
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

/// Parse a string array parameter from tool parameters.
pub(crate) fn parse_string_array(params: &Value, key: &str) -> Option<Vec<String>> {
    params.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}
