use anyhow::{Context, Result};
use hackpi_guardrails::ProfileToolAccess;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::task::Task;

// ── ToolAccess ──────────────────────────────────────────────────────────────

/// Per-tool access rule within an agent profile.
///
/// This is a type alias for `ProfileToolAccess` (defined in `hackpi-guardrails`)
/// to avoid maintaining two identical enums. The `ToolAccess` name is kept
/// as the public-facing alias for ergonomics in profile YAML definitions.
pub type ToolAccess = ProfileToolAccess;

// ── MergeStrategy ───────────────────────────────────────────────────────────

/// How a profile's system prompt template combines with the base system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeStrategy {
    /// Prepend the profile template before the base system prompt.
    Append,
    /// Use only the profile template; discard the base system prompt.
    Replace,
}

impl Default for MergeStrategy {
    fn default() -> Self {
        Self::Append
    }
}

// ── AgentProfileTransitions ─────────────────────────────────────────────────

/// Transition metadata that specifies which task states this profile is active
/// for. An empty `states` list means the profile matches any state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentProfileTransitions {
    /// Task states this profile should be active for.
    /// Empty means "match all states".
    pub states: Vec<String>,
}

impl AgentProfileTransitions {
    /// Format the states list for display.
    /// Returns `"all"` when empty, otherwise comma-joined state names.
    pub fn display_states(&self) -> String {
        if self.states.is_empty() {
            "all".to_string()
        } else {
            self.states.join(", ")
        }
    }
}

// ── AgentProfile ────────────────────────────────────────────────────────────

/// An agent behavior profile that controls system prompt, tool access, and turn
/// limits when working on a task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentProfile {
    /// Unique profile name (e.g., "default", "researcher", "coder").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// System prompt template with `{{variable}}` placeholders.
    pub system_prompt_template: String,
    /// Per-tool access rules.
    #[serde(default)]
    pub tool_access: HashMap<String, ToolAccess>,
    /// Maximum agent loop turns for this profile.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    /// How this profile's prompt merges with the base system prompt.
    #[serde(default)]
    pub merge_strategy: MergeStrategy,
    /// States this profile is active for.
    #[serde(default)]
    pub transitions: AgentProfileTransitions,
}

fn default_max_turns() -> u32 {
    25
}

// ── Default Profile ─────────────────────────────────────────────────────────

pub const DEFAULT_PROFILE_YAML: &str = r#"name: default
description: "Default agent profile — full tool access, standard behavior"
system_prompt_template: |
  You are hackpi, a coding agent. You are currently working on task {{task_id}}: "{{task_title}}".
  Current state: {{task_state}}.

  Available tools: {{tool_list}}

tool_access:
  read: allow
  search_grep: allow
  write: allow
  edit: allow
  bash: ask
  git_read: allow
  git_write: ask
  github: ask
  task: allow

max_turns: 25
merge_strategy: append

transitions:
  states: []
"#;

pub const RESEARCHER_PROFILE_YAML: &str = r#"name: researcher
description: "Read-only agent for research and planning phases"
system_prompt_template: |
  You are hackpi in research mode. You are gathering information and planning
  before making changes. Task {{task_id}}: "{{task_title}}".
  State: {{task_state}}.

  DO NOT write or edit any files. Only read and search.
  When done researching, update the task description with your findings
  and transition the task to in_progress.

tool_access:
  read: allow
  search_grep: allow
  write: deny
  edit: deny
  bash: deny
  git_read: allow
  git_write: deny
  github: allow
  task: allow

max_turns: 15
merge_strategy: append

transitions:
  states: [todo]
"#;

pub const CODER_PROFILE_YAML: &str = r#"name: coder
description: "Implementation agent with write access"
system_prompt_template: |
  You are hackpi in coding mode. You are implementing changes for task {{task_id}}: "{{task_title}}".
  State: {{task_state}}.

  Focus on writing code and running tests. When done, transition the task to in_review.

tool_access:
  read: allow
  search_grep: allow
  write: allow
  edit: allow
  bash: allow
  git_read: allow
  git_write: allow
  github: deny
  task: allow

max_turns: 40
merge_strategy: append

transitions:
  states: [in_progress]
"#;

pub const REVIEWER_PROFILE_YAML: &str = r#"name: reviewer
description: "Review agent for examining and commenting on code"
system_prompt_template: |
  You are hackpi in review mode. You are reviewing the implementation for task {{task_id}}: "{{task_title}}".
  State: {{task_state}}.

  Focus on reading code, running tests, and providing feedback.
  Create PRs or comments as needed. When the review is complete, transition to done or back to in_progress.

tool_access:
  read: allow
  search_grep: allow
  write: deny
  edit: deny
  bash: allow
  git_read: allow
  git_write: allow
  github: allow
  task: allow

max_turns: 20
merge_strategy: append

transitions:
  states: [in_review]
"#;

/// Cached default profile, parsed once.
static DEFAULT_PROFILE: LazyLock<AgentProfile> = LazyLock::new(|| {
    AgentProfile::parse_yaml(DEFAULT_PROFILE_YAML)
        .expect("built-in default profile YAML should always be valid")
});

// ── AgentProfile: core methods ──────────────────────────────────────────────

impl AgentProfile {
    /// Returns the built-in default agent profile.
    pub fn default_profile() -> Self {
        DEFAULT_PROFILE.clone()
    }

    /// Returns a HashMap of all four built-in profiles (default, researcher,
    /// coder, reviewer), parsed from embedded YAML constants.
    pub fn built_in_profiles() -> HashMap<String, AgentProfile> {
        let mut profiles = HashMap::new();
        let built_ins = [
            DEFAULT_PROFILE_YAML,
            RESEARCHER_PROFILE_YAML,
            CODER_PROFILE_YAML,
            REVIEWER_PROFILE_YAML,
        ];
        for yaml in built_ins {
            match Self::parse_yaml(yaml) {
                Ok(p) => {
                    profiles.insert(p.name.clone(), p);
                }
                Err(e) => {
                    tracing::error!("Built-in agent profile failed to parse: {e}");
                }
            }
        }
        profiles
    }

    /// Parse a profile from a YAML string.
    pub fn parse_yaml(yaml: &str) -> Result<Self> {
        let profile: AgentProfile =
            serde_yml::from_str(yaml).with_context(|| "parsing agent profile YAML")?;
        Ok(profile)
    }

    /// Return the comma-separated list of tool names that have `Allow` access
    /// in this profile.
    pub fn allowed_tools(&self) -> Vec<&str> {
        let mut tools: Vec<&str> = self
            .tool_access
            .iter()
            .filter(|(_, access)| **access == ToolAccess::Allow)
            .map(|(name, _)| name.as_str())
            .collect();
        tools.sort();
        tools
    }

    /// Perform template variable substitution on the system prompt template.
    ///
    /// Supported variables:
    /// - `{{task_id}}`
    /// - `{{task_title}}`
    /// - `{{task_state}}`
    /// - `{{task_description}}`
    /// - `{{tool_list}}`
    pub fn resolve_for_task(&self, task: &Task) -> String {
        let tool_list = self.allowed_tools().join(", ");
        let mut result = self.system_prompt_template.clone();
        result = result.replace("{{task_id}}", &task.id);
        result = result.replace("{{task_title}}", &task.title);
        result = result.replace("{{task_state}}", &task.state);
        result = result.replace("{{task_description}}", &task.description);
        result = result.replace("{{tool_list}}", &tool_list);
        result
    }

    /// Resolve the applicable agent profile for a task given its workflow
    /// and current state.
    ///
    /// Resolution order (highest priority first):
    /// 1. **Explicit override**: `workflow_profile.agent_profile` names a
    ///    profile that exists in `profiles`.
    /// 2. **State match**: any profile whose `transitions.states` contains
    ///    `task_state`. If multiple match, the first alphabetically by name
    ///    wins (deterministic).
    /// 3. **Default**: the built-in default profile.
    pub fn resolve_profile<'a>(
        profiles: &'a HashMap<String, AgentProfile>,
        workflow_profile: &crate::workflow::WorkflowProfile,
        task_state: &str,
    ) -> &'a AgentProfile {
        // Step 1: explicit workflow override
        if let Some(ref profile_name) = workflow_profile.agent_profile {
            if let Some(profile) = profiles.get(profile_name) {
                tracing::debug!(
                    "Resolved agent profile \"{}\" via workflow explicit override",
                    profile_name
                );
                return profile;
            }
            tracing::warn!(
                "Workflow references agent profile \"{profile_name}\" but it was not found, falling back"
            );
        }

        // Step 2: state matching
        if let Some(matched) = profiles
            .values()
            .filter(|p| {
                !p.transitions.states.is_empty()
                    && p.transitions.states.iter().any(|s| s == task_state)
            })
            .min_by_key(|p| &p.name)
        {
            tracing::debug!(
                "Resolved agent profile \"{}\" via state match for \"{task_state}\"",
                matched.name
            );
            return matched;
        }

        // Step 3: fall back to default
        tracing::debug!("No matching agent profile, using built-in default");
        &DEFAULT_PROFILE
    }

    /// Assemble the final system prompt by combining this profile's resolved
    /// template with the base system prompt according to the merge strategy.
    ///
    /// - `MergeStrategy::Append`: prepend the profile template before the base.
    /// - `MergeStrategy::Replace`: use only the profile template.
    pub fn assemble_system_prompt(&self, task: &Task, base_prompt: &str) -> String {
        let resolved = self.resolve_for_task(task);
        match self.merge_strategy {
            MergeStrategy::Append => {
                format!("{resolved}\n\n---\n\n{base_prompt}")
            }
            MergeStrategy::Replace => resolved,
        }
    }

    /// Return the tool access map suitable for passing to
    /// `GuardEvaluator::check_tool_with_profile`.
    pub fn as_profile_access_map(&self) -> HashMap<String, ProfileToolAccess> {
        self.tool_access.clone()
    }

    /// Load all agent profiles from `.yaml`/`.yml` files in the given
    /// directory. Returns a HashMap keyed by profile name.
    ///
    /// Invalid files are logged as warnings and skipped (not fatal).
    /// If the directory does not exist, returns an empty map (callers
    /// should fall back to `default_profile()`).
    pub async fn load_from_dir(dir: &Path) -> Result<HashMap<String, AgentProfile>> {
        let mut profiles = HashMap::new();

        if !dir.exists() {
            return Ok(profiles);
        }

        let mut entries = tokio::fs::read_dir(dir)
            .await
            .with_context(|| format!("reading agent profile directory {}", dir.display()))?;

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
                    tracing::warn!("Failed to read agent profile {}: {e}", path.display());
                    continue;
                }
            };

            let profile = match Self::parse_yaml(&content) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Failed to parse agent profile {}: {e}", path.display());
                    continue;
                }
            };

            if profile.name.is_empty() {
                tracing::warn!(
                    "Skipping agent profile with empty name in {}",
                    path.display()
                );
                continue;
            }

            tracing::debug!(
                "Loaded agent profile \"{}\" from {}",
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
    use crate::task::TaskPriority;
    use chrono::Utc;

    fn sample_task() -> Task {
        Task {
            id: "TSK-001".to_string(),
            title: "Implement feature X".to_string(),
            description: "Build the new feature".to_string(),
            state: "in_progress".to_string(),
            priority: TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── ToolAccess Tests ────────────────────────────────────────────────

    #[test]
    fn tool_access_serde_roundtrip() {
        for access in [ToolAccess::Allow, ToolAccess::Deny, ToolAccess::Ask] {
            let yaml = serde_yml::to_string(&access).expect("serialize");
            let back: ToolAccess = serde_yml::from_str(&yaml).expect("deserialize");
            assert_eq!(access, back, "roundtrip failed for {access:?}");
        }
    }

    #[test]
    fn tool_access_renames_lowercase() {
        let yaml = serde_yml::to_string(&ToolAccess::Allow).expect("serialize");
        assert!(yaml.contains("allow"));
        let yaml = serde_yml::to_string(&ToolAccess::Deny).expect("serialize");
        assert!(yaml.contains("deny"));
        let yaml = serde_yml::to_string(&ToolAccess::Ask).expect("serialize");
        assert!(yaml.contains("ask"));
    }

    #[test]
    fn tool_access_deserialize_from_yaml() {
        let allow: ToolAccess = serde_yml::from_str("allow").expect("parse");
        assert_eq!(allow, ToolAccess::Allow);
        let deny: ToolAccess = serde_yml::from_str("deny").expect("parse");
        assert_eq!(deny, ToolAccess::Deny);
        let ask: ToolAccess = serde_yml::from_str("ask").expect("parse");
        assert_eq!(ask, ToolAccess::Ask);
    }

    // ── MergeStrategy Tests ─────────────────────────────────────────────

    #[test]
    fn merge_strategy_default_is_append() {
        assert_eq!(MergeStrategy::default(), MergeStrategy::Append);
    }

    #[test]
    fn merge_strategy_serde_roundtrip() {
        for strategy in [MergeStrategy::Append, MergeStrategy::Replace] {
            let yaml = serde_yml::to_string(&strategy).expect("serialize");
            let back: MergeStrategy = serde_yml::from_str(&yaml).expect("deserialize");
            assert_eq!(strategy, back);
        }
    }

    // ── Default Profile Tests ───────────────────────────────────────────

    #[test]
    fn default_profile_has_correct_name() {
        let p = AgentProfile::default_profile();
        assert_eq!(p.name, "default");
    }

    #[test]
    fn default_profile_has_correct_description() {
        let p = AgentProfile::default_profile();
        assert_eq!(
            p.description,
            "Default agent profile — full tool access, standard behavior"
        );
    }

    #[test]
    fn default_profile_max_turns_is_25() {
        let p = AgentProfile::default_profile();
        assert_eq!(p.max_turns, 25);
    }

    #[test]
    fn default_profile_merge_strategy_is_append() {
        let p = AgentProfile::default_profile();
        assert_eq!(p.merge_strategy, MergeStrategy::Append);
    }

    #[test]
    fn default_profile_tool_access() {
        let p = AgentProfile::default_profile();
        assert_eq!(p.tool_access.get("read"), Some(&ToolAccess::Allow));
        assert_eq!(p.tool_access.get("bash"), Some(&ToolAccess::Ask));
        assert_eq!(p.tool_access.get("write"), Some(&ToolAccess::Allow));
    }

    #[test]
    fn default_profile_transitions_states_empty() {
        let p = AgentProfile::default_profile();
        assert!(p.transitions.states.is_empty());
    }

    #[test]
    fn default_profile_allowed_tools() {
        let p = AgentProfile::default_profile();
        let tools = p.allowed_tools();
        assert!(tools.contains(&"edit"));
        assert!(tools.contains(&"git_read"));
        assert!(tools.contains(&"read"));
        assert!(tools.contains(&"search_grep"));
        assert!(tools.contains(&"task"));
        assert!(tools.contains(&"write"));
        // bash, git_write, github are "ask", not "allow"
        assert!(!tools.contains(&"bash"));
        assert!(!tools.contains(&"git_write"));
        assert!(!tools.contains(&"github"));
    }

    // ── YAML Parsing Tests ─────────────────────────────────────────────

    #[test]
    fn parse_yaml_valid_profile() {
        let yaml = r#"
name: researcher
description: "Research-focused profile"
system_prompt_template: "You are a researcher working on {{task_id}}."
tool_access:
  read: allow
  search_grep: allow
  bash: deny
max_turns: 10
merge_strategy: replace
transitions:
  states:
    - todo
"#;
        let p = AgentProfile::parse_yaml(yaml).expect("parse");
        assert_eq!(p.name, "researcher");
        assert_eq!(p.max_turns, 10);
        assert_eq!(p.merge_strategy, MergeStrategy::Replace);
        assert_eq!(p.transitions.states, vec!["todo"]);
        assert_eq!(p.tool_access.get("bash"), Some(&ToolAccess::Deny));
    }

    #[test]
    fn parse_yaml_minimal_profile() {
        let yaml = r#"
name: minimal
description: "Minimal profile"
system_prompt_template: "Hello"
"#;
        let p = AgentProfile::parse_yaml(yaml).expect("parse");
        assert_eq!(p.name, "minimal");
        assert_eq!(p.max_turns, 25); // default
        assert_eq!(p.merge_strategy, MergeStrategy::Append); // default
        assert!(p.tool_access.is_empty()); // default empty
        assert!(p.transitions.states.is_empty()); // default empty
    }

    #[test]
    fn parse_yaml_invalid_yaml() {
        let yaml = "this is not: valid: yaml: [[[";
        let result = AgentProfile::parse_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_yaml_missing_name() {
        let yaml = r#"
description: "No name"
system_prompt_template: "Hello"
"#;
        let result = AgentProfile::parse_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn default_profile_yaml_roundtrip() {
        let p = AgentProfile::default_profile();
        let yaml = serde_yml::to_string(&p).expect("serialize");
        let back = AgentProfile::parse_yaml(&yaml).expect("parse");
        assert_eq!(p, back);
    }

    // ── Template Substitution Tests ─────────────────────────────────────

    #[test]
    fn resolve_for_task_substitutes_all_variables() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "Task {{task_id}}: {{task_title}} state={{task_state}} desc={{task_description}} tools={{tool_list}}".to_string(),
            tool_access: {
                let mut m = HashMap::new();
                m.insert("read".to_string(), ToolAccess::Allow);
                m.insert("write".to_string(), ToolAccess::Allow);
                m.insert("bash".to_string(), ToolAccess::Deny);
                m
            },
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions::default(),
        };
        let task = sample_task();
        let resolved = p.resolve_for_task(&task);
        assert!(
            resolved.contains("Task TSK-001:"),
            "should contain task id"
        );
        assert!(
            resolved.contains("Implement feature X"),
            "should contain task title"
        );
        assert!(
            resolved.contains("state=in_progress"),
            "should contain task state"
        );
        assert!(
            resolved.contains("desc=Build the new feature"),
            "should contain task description"
        );
        // tools sorted alphabetically: read, write (bash is denied)
        assert!(
            resolved.contains("tools=read, write"),
            "should contain allowed tools list, got: {resolved}"
        );
    }

    #[test]
    fn resolve_for_task_empty_description() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "desc={{task_description}}".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions::default(),
        };
        let mut task = sample_task();
        task.description = String::new();
        let resolved = p.resolve_for_task(&task);
        assert_eq!(resolved, "desc=");
    }

    #[test]
    fn resolve_for_task_no_allowed_tools() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "tools={{tool_list}}".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions::default(),
        };
        let task = sample_task();
        let resolved = p.resolve_for_task(&task);
        assert_eq!(resolved, "tools=");
    }

    #[test]
    fn resolve_for_task_preserves_unrecognized_placeholders() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "{{task_id}} {{unknown_var}}".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions::default(),
        };
        let task = sample_task();
        let resolved = p.resolve_for_task(&task);
        assert!(resolved.contains("TSK-001"));
        assert!(resolved.contains("{{unknown_var}}"));
    }

    // ── load_from_dir Tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn load_from_dir_nonexistent_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nonexistent");
        let profiles = AgentProfile::load_from_dir(&missing)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_empty_dir_returns_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");
        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_loads_yaml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        let yaml = r#"
name: coder
description: "Coding profile"
system_prompt_template: "You are a coder."
tool_access:
  read: allow
  write: allow
  edit: allow
  bash: allow
max_turns: 15
"#;
        let file_path = agents_dir.join("coder.yaml");
        tokio::fs::write(&file_path, yaml).await.expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("coder"));
        assert_eq!(profiles["coder"].max_turns, 15);
    }

    #[tokio::test]
    async fn load_from_dir_loads_yml_extension() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        let yaml = r#"
name: brief
description: "Brief profile"
system_prompt_template: "Brief."
"#;
        let file_path = agents_dir.join("brief.yml");
        tokio::fs::write(&file_path, yaml).await.expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("brief"));
    }

    #[tokio::test]
    async fn load_from_dir_ignores_non_yaml_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        tokio::fs::write(agents_dir.join("readme.txt"), "not a profile")
            .await
            .expect("write");
        tokio::fs::write(agents_dir.join("config.json"), "{}")
            .await
            .expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty());
    }

    #[tokio::test]
    async fn load_from_dir_skips_invalid_yaml_gracefully() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        tokio::fs::write(agents_dir.join("bad.yaml"), "not: valid: yaml: [[")
            .await
            .expect("write");

        let valid_yaml = r#"
name: good
description: "Good profile"
system_prompt_template: "Good."
"#;
        tokio::fs::write(agents_dir.join("good.yaml"), valid_yaml)
            .await
            .expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("good"));
    }

    #[tokio::test]
    async fn load_from_dir_skips_empty_name_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        let yaml = r#"
name: ""
description: "Empty name"
system_prompt_template: "Hello"
"#;
        tokio::fs::write(agents_dir.join("empty.yaml"), yaml)
            .await
            .expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert!(profiles.is_empty(), "empty-name profiles should be skipped");
    }

    #[tokio::test]
    async fn load_from_dir_loads_multiple_profiles() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        let yaml1 = r#"
name: coder
description: "Coder"
system_prompt_template: "Code."
"#;
        let yaml2 = r#"
name: researcher
description: "Researcher"
system_prompt_template: "Research."
"#;
        tokio::fs::write(agents_dir.join("coder.yaml"), yaml1)
            .await
            .expect("write");
        tokio::fs::write(agents_dir.join("researcher.yml"), yaml2)
            .await
            .expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        assert_eq!(profiles.len(), 2);
        assert!(profiles.contains_key("coder"));
        assert!(profiles.contains_key("researcher"));
    }

    // ── Built-in Profile Tests ─────────────────────────────────────────

    #[test]
    fn built_in_profiles_returns_four() {
        let profiles = AgentProfile::built_in_profiles();
        assert_eq!(profiles.len(), 4);
        assert!(profiles.contains_key("default"));
        assert!(profiles.contains_key("researcher"));
        assert!(profiles.contains_key("coder"));
        assert!(profiles.contains_key("reviewer"));
    }

    #[test]
    fn researcher_profile_is_read_only() {
        let profiles = AgentProfile::built_in_profiles();
        let researcher = &profiles["researcher"];
        assert_eq!(researcher.max_turns, 15);
        assert_eq!(researcher.transitions.states, vec!["todo"]);
        assert_eq!(researcher.tool_access.get("write"), Some(&ToolAccess::Deny));
        assert_eq!(researcher.tool_access.get("edit"), Some(&ToolAccess::Deny));
        assert_eq!(researcher.tool_access.get("bash"), Some(&ToolAccess::Deny));
        assert_eq!(researcher.tool_access.get("read"), Some(&ToolAccess::Allow));
    }

    #[test]
    fn coder_profile_has_full_write_access() {
        let profiles = AgentProfile::built_in_profiles();
        let coder = &profiles["coder"];
        assert_eq!(coder.max_turns, 40);
        assert_eq!(coder.transitions.states, vec!["in_progress"]);
        assert_eq!(coder.tool_access.get("write"), Some(&ToolAccess::Allow));
        assert_eq!(coder.tool_access.get("bash"), Some(&ToolAccess::Allow));
        assert_eq!(coder.tool_access.get("github"), Some(&ToolAccess::Deny));
    }

    #[test]
    fn reviewer_profile_can_github_but_not_write() {
        let profiles = AgentProfile::built_in_profiles();
        let reviewer = &profiles["reviewer"];
        assert_eq!(reviewer.max_turns, 20);
        assert_eq!(reviewer.transitions.states, vec!["in_review"]);
        assert_eq!(reviewer.tool_access.get("github"), Some(&ToolAccess::Allow));
        assert_eq!(reviewer.tool_access.get("write"), Some(&ToolAccess::Deny));
    }

    #[test]
    fn all_built_in_profiles_yaml_roundtrip() {
        let profiles = AgentProfile::built_in_profiles();
        for (name, profile) in &profiles {
            let yaml = serde_yml::to_string(profile).unwrap_or_else(|e| panic!("Failed to serialize {name}: {e}"));
            let back = AgentProfile::parse_yaml(&yaml)
                .unwrap_or_else(|e| panic!("Failed to parse {name}: {e}"));
            assert_eq!(
                profile, &back,
                "Roundtrip failed for built-in profile {name}"
            );
        }
    }

    // ── resolve_profile Tests ──────────────────────────────────────────

    fn make_workflow(profile_name: Option<&str>) -> crate::workflow::WorkflowProfile {
        crate::workflow::WorkflowProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            states: vec!["todo".to_string(), "in_progress".to_string(), "done".to_string()],
            transitions: vec![],
            agent_profile: profile_name.map(|s| s.to_string()),
        }
    }

    fn make_profiles() -> HashMap<String, AgentProfile> {
        let researcher = AgentProfile {
            name: "researcher".to_string(),
            description: "Researcher".to_string(),
            system_prompt_template: "Research.".to_string(),
            tool_access: HashMap::new(),
            max_turns: 10,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions {
                states: vec!["todo".to_string()],
            },
        };
        let coder = AgentProfile {
            name: "coder".to_string(),
            description: "Coder".to_string(),
            system_prompt_template: "Code.".to_string(),
            tool_access: HashMap::new(),
            max_turns: 15,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions {
                states: vec!["in_progress".to_string()],
            },
        };
        let reviewer = AgentProfile {
            name: "reviewer".to_string(),
            description: "Reviewer".to_string(),
            system_prompt_template: "Review.".to_string(),
            tool_access: HashMap::new(),
            max_turns: 20,
            merge_strategy: MergeStrategy::Replace,
            transitions: AgentProfileTransitions {
                states: vec!["in_review".to_string()],
            },
        };
        let mut map = HashMap::new();
        map.insert("researcher".to_string(), researcher);
        map.insert("coder".to_string(), coder);
        map.insert("reviewer".to_string(), reviewer);
        map
    }

    #[test]
    fn resolve_state_match_todo() {
        let profiles = make_profiles();
        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "todo");
        assert_eq!(resolved.name, "researcher");
    }

    #[test]
    fn resolve_state_match_in_progress() {
        let profiles = make_profiles();
        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "in_progress");
        assert_eq!(resolved.name, "coder");
    }

    #[test]
    fn resolve_state_match_in_review() {
        let profiles = make_profiles();
        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "in_review");
        assert_eq!(resolved.name, "reviewer");
    }

    #[test]
    fn resolve_falls_back_to_default_for_unknown_state() {
        let profiles = make_profiles();
        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "done");
        assert_eq!(resolved.name, "default");
    }

    #[test]
    fn resolve_explicit_workflow_override_takes_priority() {
        let profiles = make_profiles();
        // Task is in_progress (coder would match), but workflow says "researcher"
        let wf = make_workflow(Some("researcher"));
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "in_progress");
        assert_eq!(resolved.name, "researcher");
    }

    #[test]
    fn resolve_missing_explicit_profile_falls_through_to_state_match() {
        let profiles = make_profiles();
        // "nonexistent" profile doesn't exist; should fall through to state match
        let wf = make_workflow(Some("nonexistent"));
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "in_progress");
        assert_eq!(resolved.name, "coder");
    }

    #[test]
    fn resolve_missing_explicit_and_no_state_match_falls_to_default() {
        let profiles = make_profiles();
        let wf = make_workflow(Some("nonexistent"));
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "done");
        assert_eq!(resolved.name, "default");
    }

    #[test]
    fn resolve_empty_profiles_falls_to_default() {
        let profiles: HashMap<String, AgentProfile> = HashMap::new();
        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "todo");
        assert_eq!(resolved.name, "default");
    }

    #[test]
    fn resolve_multiple_state_matches_picks_first_alphabetically() {
        let mut profiles = make_profiles();
        // Add a second profile matching "todo" — "aaa" comes before "researcher"
        let early = AgentProfile {
            name: "aaa_researcher".to_string(),
            description: "Early".to_string(),
            system_prompt_template: "Early.".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions {
                states: vec!["todo".to_string()],
            },
        };
        profiles.insert("aaa_researcher".to_string(), early);

        let wf = make_workflow(None);
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "todo");
        assert_eq!(resolved.name, "aaa_researcher");
    }

    #[test]
    fn resolve_profile_with_empty_transitions_states_never_matches_by_state() {
        let mut profiles = make_profiles();
        // Add a profile with empty states (matches nothing by state)
        let catchall = AgentProfile {
            name: "catchall".to_string(),
            description: "Catchall".to_string(),
            system_prompt_template: "Catchall.".to_string(),
            tool_access: HashMap::new(),
            max_turns: 30,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions { states: vec![] },
        };
        profiles.insert("catchall".to_string(), catchall);

        let wf = make_workflow(None);
        // "done" doesn't match any specific state
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, "done");
        assert_eq!(resolved.name, "default");
    }

    // ── assemble_system_prompt Tests ──────────────────────────────────

    #[test]
    fn assemble_append_strategy() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "Profile: {{task_id}}".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Append,
            transitions: AgentProfileTransitions::default(),
        };
        let task = sample_task();
        let result = p.assemble_system_prompt(&task, "Base prompt");
        assert!(result.starts_with("Profile: TSK-001"));
        assert!(result.contains("\n\n---\n\n"));
        assert!(result.ends_with("Base prompt"));
    }

    #[test]
    fn assemble_replace_strategy() {
        let p = AgentProfile {
            name: "test".to_string(),
            description: "test".to_string(),
            system_prompt_template: "Profile: {{task_id}}".to_string(),
            tool_access: HashMap::new(),
            max_turns: 5,
            merge_strategy: MergeStrategy::Replace,
            transitions: AgentProfileTransitions::default(),
        };
        let task = sample_task();
        let result = p.assemble_system_prompt(&task, "Base prompt");
        assert_eq!(result, "Profile: TSK-001");
        assert!(!result.contains("Base prompt"));
    }

    #[tokio::test]
    async fn load_from_dir_last_file_wins_on_duplicate_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agents_dir = dir.path().join("agents");
        tokio::fs::create_dir_all(&agents_dir)
            .await
            .expect("create dir");

        let yaml1 = r#"
name: shared
description: "First"
system_prompt_template: "First."
max_turns: 5
"#;
        let yaml2 = r#"
name: shared
description: "Second"
system_prompt_template: "Second."
max_turns: 10
"#;
        tokio::fs::write(agents_dir.join("a_shared.yaml"), yaml1)
            .await
            .expect("write");
        tokio::fs::write(agents_dir.join("b_shared.yaml"), yaml2)
            .await
            .expect("write");

        let profiles = AgentProfile::load_from_dir(&agents_dir)
            .await
            .expect("load");
        // Hash map — last insert wins, but directory iteration order is not
        // guaranteed. We only assert exactly one entry exists.
        assert_eq!(profiles.len(), 1);
        assert!(profiles.contains_key("shared"));
    }

    // ── Profile Lifecycle Integration Test ─────────────────────────────

    #[test]
    fn profile_lifecycle_create_resolve_transition_override() {
        // Step 1: Create built-in profiles
        let profiles = AgentProfile::built_in_profiles();
        assert_eq!(profiles.len(), 4, "expected 4 built-in profiles");

        // Step 2: Workflow with no explicit override
        let wf = make_workflow(None);

        // Step 3: Sample task starts in "todo"
        let mut task = sample_task();
        task.state = "todo".to_string();

        // Step 4: todo → researcher
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, &task.state);
        assert_eq!(
            resolved.name, "researcher",
            "todo state should resolve to researcher"
        );

        // Step 5: Change to in_progress
        task.state = "in_progress".to_string();

        // Step 6: in_progress → coder
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, &task.state);
        assert_eq!(
            resolved.name, "coder",
            "in_progress state should resolve to coder"
        );

        // Step 7: Change to in_review
        task.state = "in_review".to_string();

        // Step 8: in_review → reviewer
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, &task.state);
        assert_eq!(
            resolved.name, "reviewer",
            "in_review state should resolve to reviewer"
        );

        // Step 9: Change to done
        task.state = "done".to_string();

        // Step 10: done → default (no state match, fallback)
        let resolved = AgentProfile::resolve_profile(&profiles, &wf, &task.state);
        assert_eq!(
            resolved.name, "default",
            "done state should fall back to default"
        );

        // Step 11: Workflow with explicit override
        let wf_override = make_workflow(Some("coder"));

        // Step 12: Explicit override wins over state
        let resolved = AgentProfile::resolve_profile(&profiles, &wf_override, &task.state);
        assert_eq!(
            resolved.name, "coder",
            "explicit workflow override should win even for done state"
        );
    }

    // ── System Prompt Assembly Integration Test ─────────────────────────

    #[test]
    fn system_prompt_assembly_integration() {
        let profiles = AgentProfile::built_in_profiles();
        let task = sample_task();
        // task.id = "TSK-001", task.title = "Implement feature X",
        // task.state = "in_progress", task.description = "Build the new feature"

        // --- Append strategy (default profile) ---
        let default_profile = &profiles["default"];
        assert_eq!(
            default_profile.merge_strategy,
            MergeStrategy::Append,
            "default profile should use Append strategy"
        );

        let result = default_profile.assemble_system_prompt(&task, "Base system prompt");

        // Verify resolved template variables present
        assert!(
            result.contains("TSK-001"),
            "resolved prompt should contain task_id, got: {result}"
        );
        assert!(
            result.contains("Implement feature X"),
            "resolved prompt should contain task_title, got: {result}"
        );
        assert!(
            result.contains("in_progress"),
            "resolved prompt should contain task_state, got: {result}"
        );
        // The default profile template does not include {{task_description}},
        // so the description text won't appear. Verified below via the replace
        // profile that includes all placeholders.

        // Verify the template was resolved (contains all key markers)
        assert!(
            !result.contains("{{task_id}}"),
            "{{task_id}} should be substituted"
        );
        assert!(
            !result.contains("{{task_title}}"),
            "{{task_title}} should be substituted"
        );
        assert!(
            !result.contains("{{task_state}}"),
            "{{task_state}} should be substituted"
        );
        assert!(
            !result.contains("{{task_description}}"),
            "{{task_description}} should be substituted"
        );
        assert!(
            !result.contains("{{tool_list}}"),
            "{{tool_list}} should be substituted"
        );

        // Verify append separator + base prompt
        assert!(
            result.contains("\n\n---\n\nBase system prompt"),
            "append strategy should include separator + base prompt, got: {result}"
        );

        // Verify {{tool_list}} was substituted with sorted allowed tools
        let allowed = default_profile.allowed_tools();
        let tool_list_str = allowed.join(", ");
        assert!(
            result.contains(&tool_list_str),
            "resolved prompt should contain sorted tool list '{tool_list_str}', got: {result}"
        );

        // Verify the allowed tools are sorted alphabetically
        let mut sorted_check = allowed.clone();
        sorted_check.sort();
        assert_eq!(
            allowed, sorted_check,
            "allowed_tools() should return alphabetically sorted tools"
        );

        // --- Replace strategy ---
        let replace_profile = AgentProfile {
            name: "replace_test".to_string(),
            description: "Replace strategy test".to_string(),
            system_prompt_template: "Only this: {{task_id}} {{task_title}} {{task_state}} {{task_description}} {{tool_list}}".to_string(),
            tool_access: {
                let mut m = HashMap::new();
                m.insert("read".to_string(), ToolAccess::Allow);
                m.insert("bash".to_string(), ToolAccess::Deny);
                m
            },
            max_turns: 5,
            merge_strategy: MergeStrategy::Replace,
            transitions: AgentProfileTransitions::default(),
        };

        let result = replace_profile.assemble_system_prompt(&task, "Base system prompt");

        // Replace strategy: result is ONLY the resolved template
        assert_eq!(
            result,
            "Only this: TSK-001 Implement feature X in_progress Build the new feature read",
            "replace strategy should return only the resolved template"
        );
        assert!(
            !result.contains("Base system prompt"),
            "replace strategy should not include base prompt"
        );
        assert!(
            !result.contains("---"),
            "replace strategy should not include separator"
        );
    }
}
