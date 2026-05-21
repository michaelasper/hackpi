use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::workflow::transitions::Transition;

// ── WorkflowProfile ─────────────────────────────────────────────────────────

/// A workflow profile defines a state machine for task lifecycle management.
/// It specifies the valid states and the allowed transitions between them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowProfile {
    /// Unique name for this workflow (e.g., "default", "kanban").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// All valid states in this workflow.
    pub states: Vec<String>,
    /// Allowed state transitions.
    pub transitions: Vec<Transition>,
    /// Optional agent profile name associated with this workflow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
}

// ── Default Workflow ────────────────────────────────────────────────────────

/// Default workflow YAML content embedded as a const string.
pub const DEFAULT_WORKFLOW_YAML: &str = r#"name: default
description: "Standard task lifecycle"
states:
  - todo
  - in_progress
  - blocked
  - in_review
  - done
  - cancelled
transitions:
  - from: todo
    to:
      - in_progress
      - blocked
      - cancelled
  - from: in_progress
    to:
      - blocked
      - in_review
      - done
      - cancelled
  - from: blocked
    to:
      - in_progress
      - todo
      - cancelled
  - from: in_review
    to:
      - in_progress
      - done
      - cancelled
agent_profile: default
"#;

/// Cached default workflow profile, parsed once to avoid per-call YAML parsing.
static DEFAULT_WORKFLOW: LazyLock<WorkflowProfile> = LazyLock::new(|| {
    WorkflowProfile::parse_yaml(DEFAULT_WORKFLOW_YAML)
        .expect("built-in default workflow YAML should always be valid")
});

// ── WorkflowProfile: core methods ──────────────────────────────────────────

impl WorkflowProfile {
    /// Returns the built-in default workflow profile.
    pub fn default_workflow() -> Self {
        DEFAULT_WORKFLOW.clone()
    }

    /// Parse a workflow profile from a YAML string.
    pub fn parse_yaml(yaml: &str) -> Result<Self> {
        let profile: WorkflowProfile =
            serde_yml::from_str(yaml).with_context(|| "parsing workflow YAML")?;
        Ok(profile)
    }

    /// Returns the initial (first) state for this workflow profile.
    ///
    /// Falls back to `"todo"` only if the states list is empty, which should
    /// never happen for a validated workflow.
    pub fn initial_state(&self) -> &str {
        self.states.first().map(|s| s.as_str()).unwrap_or("todo")
    }
}
