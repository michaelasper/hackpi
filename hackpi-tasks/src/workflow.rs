use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ── Transition ──────────────────────────────────────────────────────────────

/// A single state transition rule: from a given state, which target states are
/// allowed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transition {
    /// Source state name.
    pub from: String,
    /// Allowed target state names.
    pub to: Vec<String>,
}

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

impl WorkflowProfile {
    /// Returns the built-in default workflow profile.
    pub fn default_workflow() -> Self {
        Self::parse_yaml(DEFAULT_WORKFLOW_YAML)
            .expect("built-in default workflow YAML should always be valid")
    }

    /// Parse a workflow profile from a YAML string.
    pub fn parse_yaml(yaml: &str) -> Result<Self> {
        let profile: WorkflowProfile =
            serde_yaml::from_str(yaml).with_context(|| "parsing workflow YAML")?;
        Ok(profile)
    }

    /// Check whether transitioning from `from_state` to `to_state` is allowed
    /// by this workflow's transition rules.
    pub fn validate_transition(&self, from_state: &str, to_state: &str) -> bool {
        // If from == to, allow (no-op transition)
        if from_state == to_state {
            return true;
        }

        self.transitions
            .iter()
            .any(|t| t.from == from_state && t.to.iter().any(|target| target == to_state))
    }

    /// Validate the structural integrity of this workflow profile.
    ///
    /// Checks:
    /// - No duplicate states
    /// - All transition source states exist in the states list
    /// - All transition target states exist in the states list
    /// - No orphan states (states that never appear in any transition)
    pub fn validate(&self) -> Result<()> {
        // Check for duplicate states
        let mut seen = HashSet::new();
        for state in &self.states {
            if !seen.insert(state.as_str()) {
                bail!("duplicate state: \"{state}\"");
            }
        }

        // Check transition sources and targets
        let state_set: HashSet<&str> = self.states.iter().map(|s| s.as_str()).collect();

        for (i, transition) in self.transitions.iter().enumerate() {
            // Source state must exist
            if !state_set.contains(transition.from.as_str()) {
                bail!(
                    "transition {}: source state \"{}\" not found in states list",
                    i,
                    transition.from
                );
            }

            // Target states must exist
            if transition.to.is_empty() {
                bail!(
                    "transition {}: from \"{}\" has no target states",
                    i,
                    transition.from
                );
            }

            for target in &transition.to {
                if !state_set.contains(target.as_str()) {
                    bail!(
                        "transition {}: target state \"{}\" not found in states list",
                        i,
                        target
                    );
                }
            }
        }

        // Check for orphan states (states not in any transition as source or target)
        let mut referenced_states: HashSet<&str> = HashSet::new();
        for transition in &self.transitions {
            referenced_states.insert(transition.from.as_str());
            for target in &transition.to {
                referenced_states.insert(target.as_str());
            }
        }

        for state in &self.states {
            if !referenced_states.contains(state.as_str()) {
                bail!("orphan state: \"{state}\" is not referenced in any transition");
            }
        }

        Ok(())
    }

    /// Load all workflow profiles from `.yaml`/`.yml` files in the given
    /// directory. Returns a HashMap keyed by workflow name.
    ///
    /// Invalid files are logged as warnings and skipped (not fatal).
    pub async fn load_from_dir(dir: &Path) -> Result<HashMap<String, WorkflowProfile>> {
        let mut profiles = HashMap::new();

        if !dir.exists() {
            return Ok(profiles);
        }

        let mut entries = tokio::fs::read_dir(dir)
            .await
            .with_context(|| format!("reading workflow directory {}", dir.display()))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| "reading directory entry")?
        {
            let path = entry.path();
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if extension != "yaml" && extension != "yml" {
                continue;
            }

            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read workflow file {}: {e}", path.display());
                    continue;
                }
            };

            let profile = match Self::parse_yaml(&content) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Failed to parse workflow file {}: {e}", path.display());
                    continue;
                }
            };

            if let Err(e) = profile.validate() {
                tracing::warn!("Invalid workflow in file {}: {e}", path.display());
                continue;
            }

            tracing::debug!(
                "Loaded workflow profile \"{}\" from {}",
                profile.name,
                path.display()
            );
            profiles.insert(profile.name.clone(), profile);
        }

        Ok(profiles)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Default Workflow Tests ───────────────────────────────────────────

    #[test]
    fn default_workflow_has_correct_name() {
        let wf = WorkflowProfile::default_workflow();
        assert_eq!(wf.name, "default");
    }

    #[test]
    fn default_workflow_has_correct_description() {
        let wf = WorkflowProfile::default_workflow();
        assert_eq!(wf.description, "Standard task lifecycle");
    }

    #[test]
    fn default_workflow_has_six_states() {
        let wf = WorkflowProfile::default_workflow();
        assert_eq!(wf.states.len(), 6);
        assert!(wf.states.contains(&"todo".to_string()));
        assert!(wf.states.contains(&"in_progress".to_string()));
        assert!(wf.states.contains(&"blocked".to_string()));
        assert!(wf.states.contains(&"in_review".to_string()));
        assert!(wf.states.contains(&"done".to_string()));
        assert!(wf.states.contains(&"cancelled".to_string()));
    }

    #[test]
    fn default_workflow_has_four_transitions() {
        let wf = WorkflowProfile::default_workflow();
        assert_eq!(wf.transitions.len(), 4);
    }

    #[test]
    fn default_workflow_agent_profile() {
        let wf = WorkflowProfile::default_workflow();
        assert_eq!(wf.agent_profile, Some("default".to_string()));
    }

    // ── Transition Validation Tests ─────────────────────────────────────

    #[test]
    fn valid_transition_todo_to_in_progress() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("todo", "in_progress"));
    }

    #[test]
    fn valid_transition_todo_to_blocked() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("todo", "blocked"));
    }

    #[test]
    fn valid_transition_todo_to_cancelled() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("todo", "cancelled"));
    }

    #[test]
    fn invalid_transition_todo_to_done() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("todo", "done"));
    }

    #[test]
    fn invalid_transition_todo_to_in_review() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("todo", "in_review"));
    }

    #[test]
    fn valid_transition_in_progress_to_blocked() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_progress", "blocked"));
    }

    #[test]
    fn valid_transition_in_progress_to_in_review() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_progress", "in_review"));
    }

    #[test]
    fn valid_transition_in_progress_to_done() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_progress", "done"));
    }

    #[test]
    fn valid_transition_in_progress_to_cancelled() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_progress", "cancelled"));
    }

    #[test]
    fn invalid_transition_in_progress_to_todo() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("in_progress", "todo"));
    }

    #[test]
    fn valid_transition_blocked_to_in_progress() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("blocked", "in_progress"));
    }

    #[test]
    fn valid_transition_blocked_to_todo() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("blocked", "todo"));
    }

    #[test]
    fn valid_transition_in_review_to_in_progress() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_review", "in_progress"));
    }

    #[test]
    fn valid_transition_in_review_to_done() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_review", "done"));
    }

    #[test]
    fn valid_transition_in_review_to_cancelled() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("in_review", "cancelled"));
    }

    #[test]
    fn invalid_transition_done_to_in_progress() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("done", "in_progress"));
    }

    #[test]
    fn invalid_transition_done_to_todo() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("done", "todo"));
    }

    #[test]
    fn invalid_transition_cancelled_to_todo() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("cancelled", "todo"));
    }

    #[test]
    fn same_state_transition_is_allowed() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate_transition("todo", "todo"));
        assert!(wf.validate_transition("done", "done"));
        assert!(wf.validate_transition("cancelled", "cancelled"));
    }

    #[test]
    fn unknown_state_transition_is_not_allowed() {
        let wf = WorkflowProfile::default_workflow();
        assert!(!wf.validate_transition("todo", "nonexistent"));
        assert!(!wf.validate_transition("nonexistent", "todo"));
    }

    // ── Validate Structural Integrity Tests ─────────────────────────────

    #[test]
    fn default_workflow_validates_ok() {
        let wf = WorkflowProfile::default_workflow();
        assert!(wf.validate().is_ok());
    }

    #[test]
    fn validate_rejects_duplicate_states() {
        let wf = WorkflowProfile {
            name: "dup".to_string(),
            description: "test".to_string(),
            states: vec![
                "todo".to_string(),
                "done".to_string(),
                "todo".to_string(), // duplicate
            ],
            transitions: vec![Transition {
                from: "todo".to_string(),
                to: vec!["done".to_string()],
            }],
            agent_profile: None,
        };
        let result = wf.validate();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("duplicate state"),
            "should report duplicate state"
        );
    }

    #[test]
    fn validate_rejects_invalid_transition_source() {
        let wf = WorkflowProfile {
            name: "bad_source".to_string(),
            description: "test".to_string(),
            states: vec!["todo".to_string(), "done".to_string()],
            transitions: vec![Transition {
                from: "nonexistent".to_string(),
                to: vec!["done".to_string()],
            }],
            agent_profile: None,
        };
        let result = wf.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in states list"),
            "should report invalid source state"
        );
    }

    #[test]
    fn validate_rejects_invalid_transition_target() {
        let wf = WorkflowProfile {
            name: "bad_target".to_string(),
            description: "test".to_string(),
            states: vec!["todo".to_string(), "done".to_string()],
            transitions: vec![Transition {
                from: "todo".to_string(),
                to: vec!["nonexistent".to_string()],
            }],
            agent_profile: None,
        };
        let result = wf.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in states list"),
            "should report invalid target state"
        );
    }

    #[test]
    fn validate_rejects_orphan_state() {
        let wf = WorkflowProfile {
            name: "orphan".to_string(),
            description: "test".to_string(),
            states: vec!["todo".to_string(), "done".to_string(), "orphan".to_string()],
            transitions: vec![Transition {
                from: "todo".to_string(),
                to: vec!["done".to_string()],
            }],
            agent_profile: None,
        };
        let result = wf.validate();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("orphan state"),
            "should report orphan state"
        );
    }

    #[test]
    fn validate_rejects_empty_transition_targets() {
        let wf = WorkflowProfile {
            name: "empty_to".to_string(),
            description: "test".to_string(),
            states: vec!["todo".to_string(), "done".to_string()],
            transitions: vec![Transition {
                from: "todo".to_string(),
                to: vec![],
            }],
            agent_profile: None,
        };
        let result = wf.validate();
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("no target states"),
            "should report empty targets"
        );
    }

    #[test]
    fn validate_accepts_minimal_valid_workflow() {
        let wf = WorkflowProfile {
            name: "minimal".to_string(),
            description: "minimal test".to_string(),
            states: vec!["start".to_string(), "end".to_string()],
            transitions: vec![Transition {
                from: "start".to_string(),
                to: vec!["end".to_string()],
            }],
            agent_profile: None,
        };
        assert!(wf.validate().is_ok());
    }

    // ── YAML Parsing Tests ──────────────────────────────────────────────

    #[test]
    fn parse_yaml_valid_workflow() {
        let yaml = r#"
name: custom
description: "Custom workflow"
states:
  - open
  - closed
transitions:
  - from: open
    to:
      - closed
"#;
        let wf = WorkflowProfile::parse_yaml(yaml).expect("parse");
        assert_eq!(wf.name, "custom");
        assert_eq!(wf.states, vec!["open", "closed"]);
        assert_eq!(wf.transitions.len(), 1);
        assert!(wf.agent_profile.is_none());
    }

    #[test]
    fn parse_yaml_with_agent_profile() {
        let yaml = r#"
name: custom
description: "Custom workflow"
states:
  - open
  - closed
transitions:
  - from: open
    to:
      - closed
agent_profile: special
"#;
        let wf = WorkflowProfile::parse_yaml(yaml).expect("parse");
        assert_eq!(wf.agent_profile, Some("special".to_string()));
    }

    #[test]
    fn parse_yaml_invalid_yaml() {
        let yaml = "this is not: valid: yaml: [[[";
        let result = WorkflowProfile::parse_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_yaml_missing_name() {
        // serde_yaml will fail if required field "name" is missing
        let yaml = r#"
description: "No name"
states: []
transitions: []
"#;
        let result = WorkflowProfile::parse_yaml(yaml);
        // This should parse but name would be missing - serde requires it
        // Actually with serde, missing required fields cause errors
        assert!(result.is_err() || result.unwrap().name.is_empty());
    }

    #[test]
    fn default_workflow_yaml_roundtrip() {
        let wf = WorkflowProfile::default_workflow();
        let yaml = serde_yaml::to_string(&wf).expect("serialize");
        let wf2 = WorkflowProfile::parse_yaml(&yaml).expect("parse");
        assert_eq!(wf, wf2);
    }

    #[test]
    fn transition_serde_roundtrip() {
        let t = Transition {
            from: "todo".to_string(),
            to: vec!["in_progress".to_string(), "done".to_string()],
        };
        let yaml = serde_yaml::to_string(&t).expect("serialize");
        let back: Transition = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(t, back);
    }

    // ── load_from_dir Tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn load_from_dir_nonexistent_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nonexistent");
        let profiles = WorkflowProfile::load_from_dir(&missing)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_empty_dir_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");
        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_loads_yaml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Write a valid workflow YAML
        let yaml = r#"
name: custom
description: "Custom workflow"
states:
  - open
  - closed
transitions:
  - from: open
    to:
      - closed
"#;
        let file_path = workflows_dir.join("custom.yaml");
        tokio::fs::write(&file_path, yaml).await.expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("custom"));
        assert_eq!(profiles["custom"].states, vec!["open", "closed"]);
    }

    #[tokio::test]
    async fn load_from_dir_loads_yml_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        let yaml = r#"
name: brief
description: "Brief workflow"
states:
  - a
  - b
transitions:
  - from: a
    to:
      - b
"#;
        let file_path = workflows_dir.join("brief.yml");
        tokio::fs::write(&file_path, yaml).await.expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("brief"));
    }

    #[tokio::test]
    async fn load_from_dir_ignores_non_yaml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Write a .txt file
        tokio::fs::write(workflows_dir.join("readme.txt"), "not a workflow")
            .await
            .expect("write");
        // Write a .json file
        tokio::fs::write(workflows_dir.join("config.json"), "{}")
            .await
            .expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_skips_invalid_yaml_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Write invalid YAML
        tokio::fs::write(workflows_dir.join("bad.yaml"), "not: valid: yaml: [[")
            .await
            .expect("write");

        // Also write a valid one
        let valid_yaml = r#"
name: good
description: "Good workflow"
states:
  - a
  - b
transitions:
  - from: a
    to:
      - b
"#;
        tokio::fs::write(workflows_dir.join("good.yaml"), valid_yaml)
            .await
            .expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("good"));
    }

    #[tokio::test]
    async fn load_from_dir_skips_invalid_workflow_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        // Valid YAML but invalid workflow (orphan state)
        let yaml = r#"
name: invalid
description: "Has orphan"
states:
  - a
  - b
  - orphan
transitions:
  - from: a
    to:
      - b
"#;
        tokio::fs::write(workflows_dir.join("invalid.yaml"), yaml)
            .await
            .expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty(), "invalid workflows should be skipped");
    }

    #[tokio::test]
    async fn load_from_dir_loads_multiple_workflows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows_dir = dir.path().join("workflows");
        tokio::fs::create_dir_all(&workflows_dir)
            .await
            .expect("create dir");

        let yaml1 = r#"
name: flow1
description: "Flow 1"
states:
  - open
  - closed
transitions:
  - from: open
    to:
      - closed
"#;
        let yaml2 = r#"
name: flow2
description: "Flow 2"
states:
  - new
  - done
transitions:
  - from: new
    to:
      - done
"#;
        tokio::fs::write(workflows_dir.join("flow1.yaml"), yaml1)
            .await
            .expect("write");
        tokio::fs::write(workflows_dir.join("flow2.yml"), yaml2)
            .await
            .expect("write");

        let profiles = WorkflowProfile::load_from_dir(&workflows_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 2);
        assert!(profiles.contains_key("flow1"));
        assert!(profiles.contains_key("flow2"));
    }

    // ── Default workflow: terminal states ───────────────────────────────

    #[test]
    fn done_is_terminal_state() {
        let wf = WorkflowProfile::default_workflow();
        // "done" has no transitions from it in the default workflow
        let from_done = wf.transitions.iter().find(|t| t.from == "done");
        assert!(
            from_done.is_none(),
            "done should be a terminal state with no outgoing transitions"
        );
    }

    #[test]
    fn cancelled_is_terminal_state() {
        let wf = WorkflowProfile::default_workflow();
        let from_cancelled = wf.transitions.iter().find(|t| t.from == "cancelled");
        assert!(
            from_cancelled.is_none(),
            "cancelled should be a terminal state with no outgoing transitions"
        );
    }
}
