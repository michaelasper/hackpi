use std::collections::HashMap;
use std::path::PathBuf;

use crate::{FileOp, GuardReason, GuardResult, GuardType, PermissionDecision, PermissionRule, SettingsPaths};

// ── Profile Tool Access ────────────────────────────────────────────────────

/// Per-tool access rule that an agent profile can specify.
///
/// This is defined in the guardrails crate (rather than in hackpi-tasks)
/// to avoid circular dependencies. The tasks crate provides a `From` impl
/// to convert its own `ToolAccess` enum into this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileToolAccess {
    Allow,
    Deny,
    Ask,
}

/// The main guard evaluation engine.
///
/// Checks tool calls against loaded permission rules and provides
/// session-level caching for user decisions.
#[derive(Debug)]
pub struct GuardEvaluator {
    /// If true, all guard checks are bypassed.
    pub(crate) god_mode: bool,
    /// Loaded permission rules.
    pub(crate) config_rules: Vec<PermissionRule>,
    /// Session-level cache of user decisions.
    session_cache: HashMap<String, PermissionDecision>,
    /// Paths to config files.
    pub(crate) settings_paths: SettingsPaths,
    /// The root directory of the workspace (used by path_guard).
    workspace_root: PathBuf,
}

impl GuardEvaluator {
    /// Create a new `GuardEvaluator`.
    ///
    /// When `god_mode` is true, all checks return `Allow` unconditionally.
    pub fn new(god_mode: bool, settings_paths: SettingsPaths) -> Self {
        let workspace_root = settings_paths
            .hackpi
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            god_mode,
            config_rules: Vec::new(),
            session_cache: HashMap::new(),
            settings_paths,
            workspace_root,
        }
    }

    /// Generate a deterministic session cache key from tool name and params.
    ///
    /// Uses the most specific parameter for key generation:
    /// - `command` for bash-like tools
    /// - `path` for file-access tools
    /// - `operation` for git_write tools
    ///
    /// This must produce the same key as `tools.rs` uses when recording
    /// user decisions so that the cache lookup in `check_tool` matches.
    pub fn session_cache_key(&self, tool_name: &str, params: &serde_json::Value) -> String {
        if let Some(cmd) = params.get("command").and_then(|v| v.as_str()) {
            format!("{}:command:{}", tool_name, cmd)
        } else if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
            format!("{}:path:{}", tool_name, path)
        } else if let Some(op) = params.get("operation").and_then(|v| v.as_str()) {
            format!("{}:op:{}", tool_name, op)
        } else {
            format!(
                "{}:params:{}",
                tool_name,
                serde_json::to_string(params).unwrap_or_default()
            )
        }
    }

    /// Check whether a tool call is allowed, with optional profile-based
    /// tool access rules prepended before normal guardrail evaluation.
    ///
    /// Profile rules are additive:
    /// - Profile `Deny` always short-circuits (wins over guardrails).
    /// - Profile `Ask` escalates to a prompt (guardrails can still deny later).
    /// - Profile `Allow` passes through to normal guardrail checks.
    ///
    /// When `profile_access` is `None`, behaves identically to `check_tool`.
    pub fn check_tool_with_profile(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        profile_name: Option<&str>,
        profile_access: Option<&HashMap<String, ProfileToolAccess>>,
    ) -> GuardResult {
        if self.god_mode {
            return GuardResult::Allow;
        }

        // Profile tool access check (prepended before all other guards)
        if let (Some(access_map), Some(name)) = (profile_access, profile_name) {
            if let Some(access) = access_map.get(tool_name) {
                match access {
                    ProfileToolAccess::Deny => {
                        return GuardResult::Deny(format!(
                            "Tool '{tool_name}' denied by agent profile '{name}'"
                        ));
                    }
                    ProfileToolAccess::Ask => {
                        return GuardResult::Ask(GuardReason {
                            guard: GuardType::AgentProfile,
                            tool: tool_name.to_string(),
                            details: format!(
                                "Profile '{name}' requires confirmation for '{tool_name}'"
                            ),
                        });
                    }
                    ProfileToolAccess::Allow => {
                        // Allow passes through to normal guardrail checks
                    }
                }
            }
        }

        // Fall through to normal guard checks
        self.check_tool(tool_name, params)
    }

    /// Check whether a tool call is allowed.
    ///
    /// Examines the `command`, `path`, and (for git_write) `operation`
    /// parameters from the tool call, routing them through the appropriate
    /// guard components:
    /// - `command` → `command_gate`
    /// - `path` → `file_protection` + `path_guard`
    /// - `operation` (git_write) → `vcs_operation_gate`
    ///
    /// Before running any guard checks, consults the session cache for a
    /// previously recorded user decision (`AllowSession` or `Deny`). If a
    /// cached decision exists, it is applied immediately without re-checking.
    ///
    /// Returns `Allow` if all guards pass, `Deny` with a reason if any
    /// guard blocks, or `Ask` with a reason if user input is needed.
    pub fn check_tool(&self, tool_name: &str, params: &serde_json::Value) -> GuardResult {
        if self.god_mode {
            return GuardResult::Allow;
        }

        // Consult session cache first — AllowSession and Deny decisions
        // recorded for this (tool, params) combination take effect immediately.
        let cache_key = self.session_cache_key(tool_name, params);
        if let Some(decision) = self.session_cache.get(&cache_key) {
            match decision {
                PermissionDecision::AllowSession => return GuardResult::Allow,
                PermissionDecision::Deny => {
                    return GuardResult::Deny("Permission denied for this session.".into());
                }
                // AllowOnce, AlwaysAllow, AlwaysDeny are never stored in
                // the session cache (see record_decision).
                _ => {}
            }
        }

        // Check command gate (bash tool)
        if let Some(command) = params.get("command").and_then(|v| v.as_str()) {
            let result = crate::command_gate::check(command, &self.config_rules, tool_name);
            if !matches!(result, GuardResult::Allow) {
                return result;
            }
        }

        // Check file protection + path guard (tools with a path parameter)
        if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
            let path = std::path::Path::new(path);

            // Determine file operation type from tool name
            let op = match tool_name {
                "read" | "search_grep" | "searchgrep" => FileOp::Read,
                "write" | "edit" => FileOp::Write,
                _ => FileOp::Read, // unknown tools default to Read
            };

            // File protection check
            let fp_result = crate::file_protection::check(path, &op, &self.config_rules, tool_name);
            if !matches!(fp_result, GuardResult::Allow) {
                return fp_result;
            }

            // Path guard check
            let pg_result = crate::path_guard::check(
                &path.to_string_lossy(),
                &self.workspace_root,
                &self.config_rules,
                tool_name,
            );
            if !matches!(pg_result, GuardResult::Allow) {
                return pg_result;
            }
        }

        // Check git_write operation gate (git_write tool)
        // Destructive operations like reset --hard, push --force, branch_delete,
        // merge, rebase, stash_pop, and checkout are guarded even when they
        // don't carry a guarded command/path parameter.
        if tool_name.eq_ignore_ascii_case("git_write") {
            if let Some(operation) = params.get("operation").and_then(|v| v.as_str()) {
                let result = crate::vcs_operation_gate::check(
                    operation,
                    params,
                    &self.config_rules,
                    tool_name,
                );
                if !matches!(result, GuardResult::Allow) {
                    return result;
                }
            }
        }

        GuardResult::Allow
    }

    /// Record a user decision in the session cache.
    ///
    /// `AllowSession` and `Deny` (without persistence) decisions are stored
    /// for the duration of the session. `AlwaysAllow` and `AlwaysDeny` are
    /// intentionally *not* cached here — they should be persisted to config.
    pub fn record_decision(&mut self, key: String, decision: PermissionDecision) {
        match decision {
            PermissionDecision::AllowSession => {
                self.session_cache.insert(key, decision);
            }
            PermissionDecision::Deny => {
                self.session_cache.insert(key, decision);
            }
            // AllowOnce: don't cache
            // AlwaysAllow / AlwaysDeny: persisted to config, not session cache
            _ => {}
        }
    }

    /// Look up a cached session decision for the given key.
    pub fn session_decision(&self, key: &str) -> Option<&PermissionDecision> {
        self.session_cache.get(key)
    }

    /// Clear all session-level decisions.
    pub fn clear_session(&mut self) {
        self.session_cache.clear();
    }

    /// Load rules from all config files.
    ///
    /// Calls `config::load_all()` to parse and merge rules from
    /// `.hackpi/guardrails.json`, `.claude/settings.json`, and
    /// `.claude/settings.local.json`, then runs validation to ensure
    /// all parsed rules are structurally sound (valid globs, known tool
    /// names, non-empty command patterns).
    ///
    /// Validation is the same check used by hot reload, so initial load
    /// and hot reload have consistent safety guarantees.
    pub fn load_rules(&mut self) -> Result<(), String> {
        let rules = crate::config::load_all(&self.settings_paths)?;
        crate::hot_reload::validate(&rules)?;
        self.config_rules = rules;
        Ok(())
    }

    /// Attempt to reload rules from config files, keeping old rules on failure.
    ///
    /// This is the validate-before-swap entry point for hot reload.
    /// Returns `Ok(())` on success, `Err(reason)` if the new config is invalid
    /// (the old rules are preserved).
    pub fn try_reload(&mut self) -> Result<(), String> {
        let new_rules = crate::config::load_all(&self.settings_paths)?;
        crate::hot_reload::validate(&new_rules)?;
        self.config_rules = new_rules;
        Ok(())
    }

    // ── Accessors for UI slash commands ───────────────────────────────────

    /// Return the number of currently loaded permission rules.
    pub fn rule_count(&self) -> usize {
        self.config_rules.len()
    }

    /// Return whether god mode is active.
    pub fn is_god_mode(&self) -> bool {
        self.god_mode
    }

    /// Return the number of entries in the session decision cache.
    pub fn session_cache_len(&self) -> usize {
        self.session_cache.len()
    }

    /// Persist an `AlwaysAllow` or `AlwaysDeny` decision to `.claude/settings.local.json`.
    ///
    /// `AlwaysAllow` entries are appended to `permissions.allow`.
    /// `AlwaysDeny` entries are appended to `permissions.deny`.
    ///
    /// Other decision variants return `Err` since they should not be persisted.
    pub fn persist_decision(
        &self,
        decision: &PermissionDecision,
        permission_string: &str,
    ) -> Result<(), String> {
        let target_array = match decision {
            PermissionDecision::AlwaysAllow => "allow",
            PermissionDecision::AlwaysDeny => "deny",
            _ => return Err("Only AlwaysAllow and AlwaysDeny decisions can be persisted".into()),
        };
        crate::interceptor::append_to_permissions(
            &self.settings_paths.claude_local,
            permission_string,
            target_array,
        )
    }

    /// Persist an `AlwaysAllow` or `AlwaysDeny` decision to `.hackpi/guardrails.json`.
    ///
    /// Same semantics as `persist_decision` but targets the project-wide config file.
    pub fn persist_to_hackpi_config(
        &self,
        decision: &PermissionDecision,
        permission_string: &str,
    ) -> Result<(), String> {
        let target_array = match decision {
            PermissionDecision::AlwaysAllow => "allow",
            PermissionDecision::AlwaysDeny => "deny",
            _ => return Err("Only AlwaysAllow and AlwaysDeny decisions can be persisted".into()),
        };
        crate::interceptor::append_to_permissions(
            &self.settings_paths.hackpi,
            permission_string,
            target_array,
        )
    }

    /// Return a reference to the settings paths.
    pub fn settings_paths(&self) -> &SettingsPaths {
        &self.settings_paths
    }
}
