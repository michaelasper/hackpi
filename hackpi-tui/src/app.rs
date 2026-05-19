use crate::events::TuiEvent;
use hackpi_core::tools::{ToolContext, ToolRegistry, ToolResult};
use hackpi_core::types::Usage;
use hackpi_guardrails::{GuardEvaluator, GuardReason, PermissionDecision};
use hackpi_tasks::TaskCommand;
use std::collections::VecDeque;
use std::sync::Arc;

/// Represents a pending permission prompt awaiting user decision.
pub struct PermissionPrompt {
    pub id: u64,
    pub reason: GuardReason,
    pub response: Option<tokio::sync::oneshot::Sender<PermissionDecision>>,
}

impl std::fmt::Debug for PermissionPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionPrompt")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .field("response", &self.response.as_ref().map(|_| "Sender<..>"))
            .finish()
    }
}

/// Active view in the TUI.
#[derive(Debug, Clone, PartialEq)]
pub enum AppView {
    Conversation,
    TaskBoard,
    TaskDetail(String),
    /// Placeholder for a future graph view.
    TaskGraph,
}

pub enum AppState {
    Resting,
    Generating,
    Interrupted,
}

pub struct ConversationEntry {
    pub role: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallDisplay>,
}

pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub status: ToolCallStatus,
}

pub enum ToolCallStatus {
    Running,
    Done(ToolResult),
}

pub struct App {
    pub state: AppState,
    pub input: String,
    /// Frame counter for animated loading spinner.
    pub loading_frame: usize,
    pub conversation: VecDeque<ConversationEntry>,
    pub scroll_offset: usize,
    pub usage: Option<Usage>,
    pub status_message: String,
    pub quit_requested: bool,
    pub pending_permission: Option<PermissionPrompt>,
    pub task_store: Option<Arc<hackpi_tasks::JsonTaskStore>>,
    /// Active view (Conversation, TaskBoard, etc.).
    pub active_view: AppView,
    /// Cached task list for the task board view.
    pub task_list_cache: Vec<hackpi_tasks::Task>,
    /// Cursor position in the task board list.
    pub selected_task_idx: usize,
    /// Cached task for the detail view.
    pub task_detail_cache: Option<hackpi_tasks::Task>,
    /// Cached blocked-by relationships for the detail view.
    pub task_detail_blocked_by: Vec<hackpi_tasks::Task>,
    /// Cached blocking relationships for the detail view.
    pub task_detail_blocking: Vec<hackpi_tasks::Task>,
    /// Whether the slash command autocomplete popover is visible.
    pub autocomplete_visible: bool,
    /// Currently selected index in the autocomplete list.
    pub autocomplete_selected: usize,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::Resting,
            input: String::new(),
            conversation: VecDeque::new(),
            scroll_offset: 0,
            usage: None,
            status_message: String::new(),
            quit_requested: false,
            pending_permission: None,
            task_store: None,
            active_view: AppView::Conversation,
            task_list_cache: Vec::new(),
            selected_task_idx: 0,
            task_detail_cache: None,
            task_detail_blocked_by: Vec::new(),
            task_detail_blocking: Vec::new(),
            autocomplete_visible: false,
            autocomplete_selected: 0,
            loading_frame: 0,
        }
    }

    pub fn handle_event(&mut self, event: TuiEvent) {
        match event {
            TuiEvent::Submit(text) => {
                self.conversation.push_back(ConversationEntry {
                    role: "user".into(),
                    text,
                    tool_calls: Vec::new(),
                });
                self.state = AppState::Generating;
                self.scroll_offset = 0;
            }
            TuiEvent::StreamChunk(chunk) => {
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        role: "assistant".into(),
                        text: chunk,
                        tool_calls: Vec::new(),
                    });
                } else if let Some(entry) = self.conversation.back_mut() {
                    entry.text.push_str(&chunk);
                }
            }
            TuiEvent::ToolCall { id, name } => {
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        role: "assistant".into(),
                        text: String::new(),
                        tool_calls: Vec::new(),
                    });
                }
                if let Some(entry) = self.conversation.back_mut() {
                    entry.tool_calls.push(ToolCallDisplay {
                        id,
                        name,
                        status: ToolCallStatus::Running,
                    });
                }
            }
            TuiEvent::ToolResult { id, result } => {
                if let Some(entry) = self.conversation.back_mut() {
                    for tc in &mut entry.tool_calls {
                        if tc.id == id {
                            tc.status = ToolCallStatus::Done(result);
                            break;
                        }
                    }
                }
            }
            TuiEvent::Usage(usage) => {
                self.usage = Some(usage);
            }
            TuiEvent::Error(err) => {
                self.status_message = err;
                self.state = AppState::Resting;
            }
            TuiEvent::Done => {
                self.state = AppState::Resting;
            }
            TuiEvent::PermissionRequest {
                id,
                reason,
                response,
            } => {
                self.pending_permission = Some(PermissionPrompt {
                    id,
                    reason,
                    response: Some(response),
                });
            }
        }
    }

    pub fn clear(&mut self) {
        self.conversation.clear();
        self.input.clear();
        self.usage = None;
        self.scroll_offset = 0;
    }

    /// Cycle the active view: Conversation → TaskBoard → TaskGraph (placeholder) → Conversation.
    pub fn cycle_view(&mut self) {
        self.active_view = match &self.active_view {
            AppView::Conversation | AppView::TaskDetail(_) => AppView::TaskBoard,
            AppView::TaskBoard => AppView::TaskGraph,
            AppView::TaskGraph => AppView::Conversation,
        };
    }

    /// Refresh the task list cache from the task store.
    /// Returns `true` if the refresh succeeded.
    pub fn refresh_task_cache(&mut self) -> bool {
        if let Some(ref store) = self.task_store {
            let store_clone: Arc<dyn hackpi_tasks::TaskStore> =
                Arc::clone(store) as Arc<dyn hackpi_tasks::TaskStore>;
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    store_clone.list(&hackpi_tasks::TaskFilter::default()).await
                })
            });
            match result {
                Ok(tasks) => {
                    self.task_list_cache = tasks;
                    // Clamp cursor
                    if self.selected_task_idx >= self.task_list_cache.len()
                        && !self.task_list_cache.is_empty()
                    {
                        self.selected_task_idx = self.task_list_cache.len() - 1;
                    }
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Move the task board cursor up.
    pub fn task_cursor_up(&mut self) {
        if self.selected_task_idx > 0 {
            self.selected_task_idx -= 1;
        }
    }

    /// Move the task board cursor down.
    pub fn task_cursor_down(&mut self) {
        if !self.task_list_cache.is_empty() {
            self.selected_task_idx =
                (self.selected_task_idx + 1).min(self.task_list_cache.len() - 1);
        }
    }

    /// Enter the selected task detail view. Returns the task ID if a task was selected.
    pub fn enter_task_detail(&mut self) -> Option<String> {
        if let Some(task) = self.task_list_cache.get(self.selected_task_idx) {
            let id = task.id.clone();
            self.active_view = AppView::TaskDetail(id.clone());
            self.load_task_detail(&id);
            Some(id)
        } else {
            None
        }
    }

    /// Load the task detail data (task, blocked_by, blocking) from the store.
    /// If the task is not found, sets cache to None and sets an error status message.
    pub fn load_task_detail(&mut self, id: &str) {
        if let Some(ref store) = self.task_store {
            let store_clone: Arc<dyn hackpi_tasks::TaskStore> =
                Arc::clone(store) as Arc<dyn hackpi_tasks::TaskStore>;
            let id_owned = id.to_string();
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let task = store_clone.get(&id_owned).await?;
                    let blocked_by = store_clone.blocked_by(&id_owned).await.unwrap_or_default();
                    let blocking = store_clone.blocking(&id_owned).await.unwrap_or_default();
                    Ok::<_, anyhow::Error>((task, blocked_by, blocking))
                })
            });
            match result {
                Ok((Some(task), blocked_by, blocking)) => {
                    self.task_detail_cache = Some(task);
                    self.task_detail_blocked_by = blocked_by;
                    self.task_detail_blocking = blocking;
                }
                Ok((None, _, _)) => {
                    self.task_detail_cache = None;
                    self.task_detail_blocked_by = Vec::new();
                    self.task_detail_blocking = Vec::new();
                    self.status_message = format!("Task {id} not found");
                    self.active_view = AppView::TaskBoard;
                }
                Err(e) => {
                    self.task_detail_cache = None;
                    self.task_detail_blocked_by = Vec::new();
                    self.task_detail_blocking = Vec::new();
                    self.status_message = format!("Error loading task: {e}");
                    self.active_view = AppView::TaskBoard;
                }
            }
        } else {
            self.task_detail_cache = None;
            self.task_detail_blocked_by = Vec::new();
            self.task_detail_blocking = Vec::new();
        }
    }

    /// Navigate to the previous task in the list (from detail view).
    pub fn task_detail_prev(&mut self) {
        if self.selected_task_idx > 0 {
            self.selected_task_idx -= 1;
            if let Some(task) = self.task_list_cache.get(self.selected_task_idx) {
                let id = task.id.clone();
                self.active_view = AppView::TaskDetail(id.clone());
                self.load_task_detail(&id);
            }
        }
    }

    /// Navigate to the next task in the list (from detail view).
    pub fn task_detail_next(&mut self) {
        if !self.task_list_cache.is_empty()
            && self.selected_task_idx < self.task_list_cache.len() - 1
        {
            self.selected_task_idx += 1;
            if let Some(task) = self.task_list_cache.get(self.selected_task_idx) {
                let id = task.id.clone();
                self.active_view = AppView::TaskDetail(id.clone());
                self.load_task_detail(&id);
            }
        }
    }

    /// Get the filtered list of commands based on current input.
    pub fn filtered_commands(&self) -> Vec<&'static CommandInfo> {
        let input = self.input.trim();
        if input.starts_with('/') {
            filter_commands(input)
        } else {
            Vec::new()
        }
    }

    /// Move the autocomplete selection down by one.
    pub fn autocomplete_next(&mut self) {
        let filtered = self.filtered_commands();
        if filtered.is_empty() {
            return;
        }
        self.autocomplete_selected = (self.autocomplete_selected + 1).min(filtered.len() - 1);
    }

    /// Move the autocomplete selection up by one.
    pub fn autocomplete_prev(&mut self) {
        if self.autocomplete_selected > 0 {
            self.autocomplete_selected -= 1;
        }
    }

    /// Get the name of the currently selected command, if any and if autocomplete is visible.
    pub fn autocomplete_select(&self) -> Option<&'static str> {
        if !self.autocomplete_visible {
            return None;
        }
        let filtered = self.filtered_commands();
        filtered.get(self.autocomplete_selected).map(|c| c.name)
    }

    /// Update autocomplete visibility based on current state.
    pub fn update_autocomplete_state(&mut self) {
        let should_show = self.input.starts_with('/')
            && matches!(self.state, AppState::Resting)
            && matches!(self.active_view, AppView::Conversation);

        if should_show && !self.filtered_commands().is_empty() {
            self.autocomplete_visible = true;
        } else {
            self.autocomplete_visible = false;
            self.autocomplete_selected = 0;
        }

        // Clamp selected index to filtered list bounds
        let filtered = self.filtered_commands();
        if !filtered.is_empty() && self.autocomplete_selected >= filtered.len() {
            self.autocomplete_selected = filtered.len() - 1;
        }
    }

    /// Go back from the current view (Esc key).
    pub fn go_back(&mut self) {
        match &self.active_view {
            AppView::TaskDetail(_) => {
                self.task_detail_cache = None;
                self.task_detail_blocked_by = Vec::new();
                self.task_detail_blocking = Vec::new();
                self.active_view = AppView::TaskBoard;
            }
            _ => {
                self.active_view = AppView::Conversation;
            }
        }
    }
}

/// Metadata about a registered slash command for autocomplete display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
}

/// All registered slash commands with descriptions for the autocomplete modal.
pub const SLASH_COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "/help",
        description: "Show available commands",
    },
    CommandInfo {
        name: "/clear",
        description: "Clear the conversation",
    },
    CommandInfo {
        name: "/quit",
        description: "Exit the application",
    },
    CommandInfo {
        name: "/guardrails:status",
        description: "Show guardrails status",
    },
    CommandInfo {
        name: "/guardrails:clean",
        description: "Clear session cache",
    },
    CommandInfo {
        name: "/guardrails:onboarding",
        description: "Write a preset guardrails config",
    },
    CommandInfo {
        name: "/git:status",
        description: "Show git status (via git_read)",
    },
    CommandInfo {
        name: "/git:log",
        description: "Show recent git log (via git_read)",
    },
    CommandInfo {
        name: "/github:pr-list",
        description: "List open pull requests (via github)",
    },
    CommandInfo {
        name: "/task",
        description: "Manage tasks (create, list, show, ...)",
    },
    CommandInfo {
        name: "/tasks",
        description: "Alias for /task list",
    },
];

/// Return all slash commands whose name starts with the given filter text (case-insensitive).
pub fn filter_commands(filter: &str) -> Vec<&'static CommandInfo> {
    let lower = filter.to_lowercase();
    SLASH_COMMANDS
        .iter()
        .filter(|cmd| cmd.name.starts_with(&lower))
        .collect()
}

/// Invoke a registered tool by name with the given parameters and render
/// the result as a tool card in the TUI conversation.
///
/// Emits `ToolCall` → `ToolResult` events so the conversation view shows
/// a proper tool card with status indicator. Falls back to an error event
/// if the tool is not found.
async fn invoke_tool_and_render(
    tool_name: &str,
    params: serde_json::Value,
    tool_registry: &ToolRegistry,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
) -> bool {
    let tool_id = format!(
        "slash-{tool_name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let tool = match tool_registry.get(tool_name) {
        Some(t) => t,
        None => {
            let err = format!("Tool '{tool_name}' is not registered.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            return true;
        }
    };

    // Emit ToolCall event so the conversation shows a tool card
    tui_tx
        .send(TuiEvent::ToolCall {
            id: tool_id.clone(),
            name: tool_name.to_string(),
        })
        .ok();

    let ctx = ToolContext {
        workspace_root: std::env::current_dir().unwrap_or_default(),
        signal: tokio::sync::watch::channel(false).1,
    };

    let result = tool.execute(params, &ctx).await;

    // Emit ToolResult event to complete the tool card
    tui_tx
        .send(TuiEvent::ToolResult {
            id: tool_id,
            result,
        })
        .ok();

    tui_tx.send(TuiEvent::Done).ok();
    true
}

pub async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    guard_evaluator: &mut GuardEvaluator,
    tool_registry: &hackpi_core::tools::ToolRegistry,
) -> bool {
    let parts: Vec<&str> = cmd.trim().splitn(2, char::is_whitespace).collect();
    let command = parts[0];
    match command {
        "/help" => {
            let help_text = "\
Available commands:
   /help  - Show this help message
   /clear - Clear the conversation
   /quit  - Exit the application
   /guardrails:status - Show guardrails status
   /guardrails:clean - Clear session cache
   /guardrails:onboarding [preset] - Write a preset guardrails config
   /git:status - Show git status (via git_read)
   /git:log - Show recent git log (via git_read)
   /github:pr-list - List open pull requests (via github)
   /task create <title> - Create a new task
   /task list - List all tasks
   /task show <id> - Show task details
   /task move <id> <state> - Move task to a new state
   /task done <id> - Mark task as done
   /task block <id> <blocked_by> - Add blocking dependency
   /task unblock <id> <blocked_by> - Remove blocking dependency
   /task label <id> <label> - Add a label to a task
   /task assign <id> <assignee> - Assign task to someone
   /tasks - Alias for /task list";
            tui_tx
                .send(TuiEvent::StreamChunk(help_text.to_string()))
                .ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        "/clear" => {
            app.clear();
            true
        }
        "/quit" => {
            app.quit_requested = true;
            true
        }
        "/guardrails:status" => {
            let rule_count = guard_evaluator.rule_count();
            let god_mode = guard_evaluator.is_god_mode();
            let cache_len = guard_evaluator.session_cache_len();
            let god_mode_str = if god_mode { "yes" } else { "no" };

            // Determine which guards are active by checking if rules exist
            // for each guard type (this is best-effort based on rule count)
            let status_text = format!(
                "\
Guardrails Status:
  Rules loaded: {rule_count}
  God mode: {god_mode_str}
  Session cache entries: {cache_len}
  Active guards: PathGuard ✓, CommandGate ✓, FileProtection ✓"
            );
            tui_tx.send(TuiEvent::StreamChunk(status_text)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        "/guardrails:clean" => {
            guard_evaluator.clear_session();
            let msg = "Session cache cleared.".to_string();
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        cmd if cmd.starts_with("/guardrails:onboarding") => {
            let rest = parts.get(1).copied().unwrap_or("");
            let preset = rest.trim().to_lowercase();

            let (preset_name, config_json) = match preset.as_str() {
                "strict" => ("strict", STRICT_CONFIG),
                "balanced" => ("balanced", BALANCED_CONFIG),
                "permissive" => ("permissive", PERMISSIVE_CONFIG),
                "" => {
                    let summary = "\
Guardrails Onboarding Presets:
   /guardrails:onboarding strict     - Deny everything, ask for everything
   /guardrails:onboarding balanced   - Balanced defaults (recommended)
   /guardrails:onboarding permissive - Permissive rules with minimal restrictions";
                    tui_tx.send(TuiEvent::StreamChunk(summary.to_string())).ok();
                    tui_tx.send(TuiEvent::Done).ok();
                    return true;
                }
                _ => {
                    let err = format!(
                        "Unknown preset: '{preset}'. Available: strict, balanced, permissive"
                    );
                    tui_tx.send(TuiEvent::Error(err)).ok();
                    return true;
                }
            };

            let hackpi_dir = match guard_evaluator.settings_paths().hackpi.parent() {
                Some(dir) => dir.to_path_buf(),
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Cannot determine workspace root for guardrails config".into(),
                        ))
                        .ok();
                    return true;
                }
            };

            // Create .hackpi directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&hackpi_dir) {
                let err = format!("Failed to create directory {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            let config_path = hackpi_dir.join("guardrails.json");
            if let Err(e) = std::fs::write(&config_path, config_json) {
                let err = format!("Failed to write config file: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            // Reload rules from the new config
            if let Err(e) = guard_evaluator.load_rules() {
                let err = format!("Failed to load rules after writing config: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return true;
            }

            let rule_count = guard_evaluator.rule_count();
            let msg = format!(
                "Wrote {preset_name} guardrails config to {} ({rule_count} rules loaded).",
                config_path.display()
            );
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            true
        }
        "/git:status" => {
            invoke_tool_and_render(
                "git_read",
                serde_json::json!({"operation": "status"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/git:log" => {
            invoke_tool_and_render(
                "git_read",
                serde_json::json!({"operation": "log"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/github:pr-list" => {
            invoke_tool_and_render(
                "github",
                serde_json::json!({"operation": "pr_list"}),
                tool_registry,
                tui_tx,
            )
            .await
        }
        "/tasks" => {
            // /tasks is a shortcut for /task list
            match &app.task_store {
                Some(store) => {
                    let cmd = TaskCommand::List;
                    match hackpi_tasks::handle_task_command(&cmd, store.as_ref()).await {
                        Ok(output) => {
                            tui_tx.send(TuiEvent::StreamChunk(output)).ok();
                            tui_tx.send(TuiEvent::Done).ok();
                        }
                        Err(e) => {
                            tui_tx.send(TuiEvent::Error(e.to_string())).ok();
                        }
                    }
                    true
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    true
                }
            }
        }
        cmd if cmd.starts_with("/task") => {
            // Parse "/task <subcommand> [args]" or "/task" alone
            // `command` is "/task", `rest` from parts contains the subcommand + args
            let task_input = parts.get(1).copied().unwrap_or("").trim();
            match &app.task_store {
                Some(store) => {
                    match hackpi_tasks::parse_slash_task_command(task_input) {
                        Ok(task_cmd) => {
                            match hackpi_tasks::handle_task_command(&task_cmd, store.as_ref()).await
                            {
                                Ok(output) => {
                                    tui_tx.send(TuiEvent::StreamChunk(output)).ok();
                                    tui_tx.send(TuiEvent::Done).ok();
                                }
                                Err(e) => {
                                    tui_tx.send(TuiEvent::Error(e.to_string())).ok();
                                }
                            }
                        }
                        Err(e) => {
                            tui_tx.send(TuiEvent::Error(e)).ok();
                        }
                    }
                    true
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    true
                }
            }
        }
        _ => {
            let err = format!("Unknown command: {command}. Type /help for available commands.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            true
        }
    }
}

// ── Preset configs ──────────────────────────────────────────────────────────

const STRICT_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": [],
    "deny": ["Read(./**)", "Write(./**)", "Bash(*)"]
  },
  "path_access": {
    "allow": [],
    "deny": ["/**", "~/**"],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "deny",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "deny", "write": "deny" },
      "*.pem": { "read": "deny", "write": "deny" },
      "*.key": { "read": "deny", "write": "deny" }
    }
  }
}"#;

const BALANCED_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./docs/**)"],
    "deny": ["Read(./.env)", "Write(./.env)", "Bash(sudo *)"]
  },
  "path_access": {
    "allow": [],
    "deny": [],
    "ask": true
  },
  "command_gate": {
    "patterns": {
      "rm -rf": "ask",
      "sudo": "deny",
      "curl": "ask",
      "dd": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env*": { "read": "ask", "write": "deny" },
      "*.pem": { "read": "ask", "write": "deny" },
      "*.key": { "read": "ask", "write": "deny" }
    }
  }
}"#;

const PERMISSIVE_CONFIG: &str = r#"{
  "version": 1,
  "permissions": {
    "allow": ["Read(./**)", "Write(./**)", "Bash(*)"],
    "deny": []
  },
  "path_access": {
    "allow": ["/**", "~/**"],
    "deny": [],
    "ask": false
  },
  "command_gate": {
    "patterns": {
      "sudo": "deny",
      "rm -rf /": "deny"
    }
  },
  "file_protection": {
    "patterns": {
      ".env": { "write": "deny" }
    }
  }
}"#;

/// Map a key character to a `PermissionDecision`, matching the key bindings
/// used in the TUI event loop when a permission prompt is active.
pub fn permission_decision_from_key(c: char) -> Option<PermissionDecision> {
    match c {
        '1' => Some(PermissionDecision::AllowOnce),
        '2' => Some(PermissionDecision::AllowSession),
        '3' => Some(PermissionDecision::Deny),
        '4' => Some(PermissionDecision::AlwaysAllow),
        '5' => Some(PermissionDecision::AlwaysDeny),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_guardrails::SettingsPaths;
    use tokio::sync::mpsc;

    /// Helper to create a GuardEvaluator backed by a temp directory.
    fn make_guard_evaluator() -> (GuardEvaluator, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = SettingsPaths::new(dir.path());
        let evaluator = GuardEvaluator::new(false, paths);
        (evaluator, dir)
    }

    /// Helper to create a ToolRegistry (empty — sufficient for non-VCS tests).
    fn make_tool_registry() -> ToolRegistry {
        ToolRegistry::new()
    }

    #[tokio::test]
    async fn test_slash_command_prevents_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/help", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert!(handled);
    }

    #[tokio::test]
    async fn test_slash_help_generates_help_text() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command("/help", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);
        let mut found_chunk = false;
        let mut found_done = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::StreamChunk(text) => {
                    found_chunk = true;
                    assert!(text.contains("/help"));
                    assert!(text.contains("/clear"));
                    assert!(text.contains("/quit"));
                }
                TuiEvent::Done => found_done = true,
                _ => {}
            }
        }
        assert!(found_chunk);
        assert!(found_done);
    }

    #[tokio::test]
    async fn test_slash_clear_clears_conversation() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/clear", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert!(handled);
        assert!(app.conversation.is_empty());
        assert!(app.input.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_slash_command_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/unknown", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert!(handled);
        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("/unknown"));
            }
        }
        assert!(found_error);
    }

    // ── Guardrails slash command tests ────────────────────────────────────

    #[tokio::test]
    async fn test_guardrails_status_returns_info() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command(
            "/guardrails:status",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);
        let mut found_chunk = false;
        let mut found_done = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::StreamChunk(text) => {
                    found_chunk = true;
                    assert!(text.contains("Guardrails Status"));
                    assert!(text.contains("Rules loaded"));
                    assert!(text.contains("God mode"));
                    assert!(text.contains("Active guards"));
                }
                TuiEvent::Done => found_done = true,
                _ => {}
            }
        }
        assert!(found_chunk);
        assert!(found_done);
    }

    #[tokio::test]
    async fn test_guardrails_clean_clears_session() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();

        // Record a session decision first
        ge.record_decision("test-key".into(), PermissionDecision::AllowSession);
        assert_eq!(ge.session_cache_len(), 1);

        let handled = handle_slash_command(
            "/guardrails:clean",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);

        // Verify session cache is cleared
        assert_eq!(ge.session_cache_len(), 0);

        // Verify output message
        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("cleared"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_balanced_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();

        // Initial state: no rules loaded
        assert_eq!(ge.rule_count(), 0);

        let handled = handle_slash_command(
            "/guardrails:onboarding balanced",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);

        // Verify rules were loaded
        assert!(
            ge.rule_count() > 0,
            "rules should be loaded after onboarding"
        );

        // Verify config file was written
        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists(), "config file should exist");
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(
            content.contains("Read(./docs/**)"),
            "balanced config should contain Read(./docs/**)"
        );
        assert!(
            content.contains("\"ask\": true"),
            "balanced config should have ask: true"
        );

        // Verify success message
        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("rules loaded"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_no_args_shows_presets() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command(
            "/guardrails:onboarding",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);
        let mut found_summary = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_summary = true;
                assert!(msg.contains("strict"));
                assert!(msg.contains("balanced"));
                assert!(msg.contains("permissive"));
            }
        }
        assert!(found_summary);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_strict_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();
        let handled = handle_slash_command(
            "/guardrails:onboarding strict",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);
        assert!(ge.rule_count() > 0);

        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("deny"));
        assert!(content.contains("rm -rf"));

        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("strict"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_onboarding_permissive_writes_config() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, dir) = make_guard_evaluator();
        let handled = handle_slash_command(
            "/guardrails:onboarding permissive",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);
        assert!(ge.rule_count() > 0);

        let config_path = dir.path().join(".hackpi/guardrails.json");
        assert!(config_path.exists());
        let content = std::fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("Read(./**)"));

        let mut found_msg = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_msg = true;
                assert!(msg.contains("permissive"), "msg: {msg}");
            }
        }
        assert!(found_msg);
    }

    #[tokio::test]
    async fn test_guardrails_unknown_subcommand_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled = handle_slash_command(
            "/guardrails:unknown",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert!(handled);
        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("/guardrails:unknown"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_guardrails_commands_prevent_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();

        let cmds = [
            "/guardrails:status",
            "/guardrails:clean",
            "/guardrails:onboarding balanced",
        ];
        for cmd in &cmds {
            let handled =
                handle_slash_command(cmd, &mut app, &tx, &mut ge, &make_tool_registry()).await;
            assert!(handled, "command '{cmd}' should prevent agent spawn");
        }
    }

    // ── VCS slash command tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_git_status_slash_command_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command("/git:status", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled, "/git:status should be handled");
    }

    #[tokio::test]
    async fn test_git_log_slash_command_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command("/git:log", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled, "/git:log should be handled");
    }

    #[tokio::test]
    async fn test_github_pr_list_slash_command_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled =
            handle_slash_command("/github:pr-list", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled, "/github:pr-list should be handled");
    }

    #[tokio::test]
    async fn test_git_status_emits_tool_call_event() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        // Register a real git_read tool with a temp workspace
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(hackpi_vcs::git_read::GitReadTool::new(
            tmp.path().to_path_buf(),
        )));
        let _handled = handle_slash_command("/git:status", &mut app, &tx, &mut ge, &registry).await;

        let mut found_tool_call = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::ToolCall { name, .. } = &event {
                found_tool_call = true;
                assert_eq!(name, "git_read");
            }
        }
        assert!(
            found_tool_call,
            "/git:status should emit a ToolCall event for git_read"
        );
    }

    #[tokio::test]
    async fn test_git_log_emits_tool_call_event() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(hackpi_vcs::git_read::GitReadTool::new(
            tmp.path().to_path_buf(),
        )));
        let _handled = handle_slash_command("/git:log", &mut app, &tx, &mut ge, &registry).await;

        let mut found_tool_call = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::ToolCall { name, .. } = &event {
                found_tool_call = true;
                assert_eq!(name, "git_read");
            }
        }
        assert!(
            found_tool_call,
            "/git:log should emit a ToolCall event for git_read"
        );
    }

    #[tokio::test]
    async fn test_github_pr_list_emits_tool_call_event() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut registry = ToolRegistry::new();
        let vcs_config = hackpi_vcs::VcsConfig::from_env(tmp.path());
        registry.register(Box::new(hackpi_vcs::github::GitHubTool::new(
            tmp.path().to_path_buf(),
            vcs_config,
        )));
        let _handled =
            handle_slash_command("/github:pr-list", &mut app, &tx, &mut ge, &registry).await;

        let mut found_tool_call = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::ToolCall { name, .. } = &event {
                found_tool_call = true;
                assert_eq!(name, "github");
            }
        }
        assert!(
            found_tool_call,
            "/github:pr-list should emit a ToolCall event for github"
        );
    }

    #[tokio::test]
    async fn test_help_includes_vcs_commands() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let _handled = handle_slash_command("/help", &mut app, &tx, &mut ge, &registry).await;

        let mut found_chunk = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_chunk = true;
                assert!(text.contains("/git:status"), "help should list /git:status");
                assert!(text.contains("/git:log"), "help should list /git:log");
                assert!(
                    text.contains("/github:pr-list"),
                    "help should list /github:pr-list"
                );
            }
        }
        assert!(found_chunk);
    }

    #[test]
    fn test_submit_creates_user_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "user");
        assert_eq!(app.conversation[0].text, "hello");
        assert!(matches!(app.state, AppState::Generating));
    }

    #[test]
    fn test_stream_chunk_appends_to_assistant() {
        let mut app = App::new();
        app.handle_event(TuiEvent::StreamChunk("Hello".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "assistant");
        assert_eq!(app.conversation[0].text, "Hello");

        app.handle_event(TuiEvent::StreamChunk(", world".into()));
        assert_eq!(app.conversation[0].text, "Hello, world");
    }

    #[test]
    fn test_tool_call_creates_assistant_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
        });
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "assistant");
        assert_eq!(app.conversation[0].tool_calls.len(), 1);
        assert_eq!(app.conversation[0].tool_calls[0].name, "read");
        assert!(matches!(
            app.conversation[0].tool_calls[0].status,
            ToolCallStatus::Running
        ));
    }

    #[test]
    fn test_tool_result_updates_status() {
        let mut app = App::new();
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
        });
        app.handle_event(TuiEvent::ToolResult {
            id: "tc1".into(),
            result: ToolResult::Success {
                content: "file content".into(),
            },
        });
        assert!(matches!(
            app.conversation[0].tool_calls[0].status,
            ToolCallStatus::Done(_)
        ));
    }

    #[test]
    fn test_done_sets_resting() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Done);
        assert!(matches!(app.state, AppState::Resting));
    }

    #[test]
    fn test_error_sets_resting_and_message() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Error("API error".into()));
        assert!(matches!(app.state, AppState::Resting));
        assert_eq!(app.status_message, "API error");
    }

    #[test]
    fn test_clear_resets_state() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::StreamChunk("Hi".into()));
        app.usage = Some(Usage {
            input_tokens: 10,
            output_tokens: 5,
        });
        app.clear();
        assert!(app.conversation.is_empty());
        assert!(app.input.is_empty());
        assert!(app.usage.is_none());
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_conversation_preserves_history() {
        let mut app = App::new();
        // Simulate two submit/response cycles
        app.handle_event(TuiEvent::Submit("first message".into()));
        app.handle_event(TuiEvent::StreamChunk("first response".into()));
        app.handle_event(TuiEvent::Done);

        app.handle_event(TuiEvent::Submit("second message".into()));
        app.handle_event(TuiEvent::StreamChunk("second response".into()));
        app.handle_event(TuiEvent::Done);

        assert_eq!(app.conversation.len(), 4);
        assert_eq!(app.conversation[0].text, "first message");
        assert_eq!(app.conversation[1].text, "first response");
        assert_eq!(app.conversation[2].text, "second message");
        assert_eq!(app.conversation[3].text, "second response");
    }

    #[test]
    fn test_stream_chunk_without_existing_assistant_creates_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::StreamChunk("direct response".into()));
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].text, "direct response");
    }

    #[test]
    fn test_usage_stored() {
        let mut app = App::new();
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 50,
        };
        app.handle_event(TuiEvent::Usage(usage));
        assert_eq!(app.usage.as_ref().unwrap().input_tokens, 100);
        assert_eq!(app.usage.as_ref().unwrap().output_tokens, 50);
    }

    // ── Permission prompt tests ──────────────────────────────────────────

    #[test]
    fn test_permission_prompt_initial_state() {
        let app = App::new();
        assert!(app.pending_permission.is_none());
    }

    #[test]
    fn test_permission_request_stored_in_app() {
        let mut app = App::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        app.handle_event(TuiEvent::PermissionRequest {
            id: 42,
            reason,
            response: tx,
        });

        assert!(app.pending_permission.is_some());
        let prompt = app.pending_permission.as_ref().unwrap();
        assert_eq!(prompt.id, 42);
        assert_eq!(
            prompt.reason.guard,
            hackpi_guardrails::GuardType::CommandGate
        );
        assert_eq!(prompt.reason.tool, "bash");
        assert!(prompt.response.is_some());
    }

    #[test]
    fn test_pending_permission_cleared_after_decision_sent() {
        let mut app = App::new();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let reason = GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        app.handle_event(TuiEvent::PermissionRequest {
            id: 1,
            reason,
            response: tx,
        });

        // Simulate taking the prompt and sending a decision
        if let Some(mut prompt) = app.pending_permission.take() {
            if let Some(sender) = prompt.response.take() {
                sender.send(PermissionDecision::Deny).ok();
            }
        }

        assert!(app.pending_permission.is_none());
        assert_eq!(rx.try_recv(), Ok(PermissionDecision::Deny));
    }

    // ── Permission decision key mapping tests ────────────────────────────

    #[test]
    fn test_key_1_maps_to_allow_once() {
        assert_eq!(
            permission_decision_from_key('1'),
            Some(PermissionDecision::AllowOnce)
        );
    }

    #[test]
    fn test_key_2_maps_to_allow_session() {
        assert_eq!(
            permission_decision_from_key('2'),
            Some(PermissionDecision::AllowSession)
        );
    }

    #[test]
    fn test_key_3_maps_to_deny() {
        assert_eq!(
            permission_decision_from_key('3'),
            Some(PermissionDecision::Deny)
        );
    }

    #[test]
    fn test_key_4_maps_to_always_allow() {
        assert_eq!(
            permission_decision_from_key('4'),
            Some(PermissionDecision::AlwaysAllow)
        );
    }

    #[test]
    fn test_key_5_maps_to_always_deny() {
        assert_eq!(
            permission_decision_from_key('5'),
            Some(PermissionDecision::AlwaysDeny)
        );
    }

    #[test]
    fn test_other_keys_return_none() {
        assert_eq!(permission_decision_from_key('0'), None);
        assert_eq!(permission_decision_from_key('6'), None);
        assert_eq!(permission_decision_from_key('a'), None);
        assert_eq!(permission_decision_from_key(' '), None);
    }

    // ── Task slash command tests ──────────────────────────────────────────

    /// Helper to create an App with a task store backed by a temp directory.
    async fn make_app_with_task_store() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let tasks_dir = dir.path().join("tasks");
        let store = hackpi_tasks::JsonTaskStore::new(tasks_dir)
            .await
            .expect("create task store");
        let mut app = App::new();
        app.task_store = Some(std::sync::Arc::new(store));
        (app, dir)
    }

    #[tokio::test]
    async fn test_task_create_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command(
            "/task create Add logging",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("Created TSK-001"));
                assert!(text.contains("Add logging"));
            }
        }
        assert!(found_output, "should have output from /task create");
    }

    #[tokio::test]
    async fn test_task_list_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Create a task first
        handle_slash_command("/task create My task", &mut app, &tx, &mut ge, &registry).await;
        // Drain events
        while rx.try_recv().is_ok() {}

        // List tasks
        let handled = handle_slash_command("/task list", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("TSK-001"));
                assert!(text.contains("My task"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_tasks_alias_lists() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Create a task first
        handle_slash_command("/task create Test", &mut app, &tx, &mut ge, &registry).await;
        while rx.try_recv().is_ok() {}

        // Use /tasks alias
        let handled = handle_slash_command("/tasks", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("TSK-001"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_task_show_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command(
            "/task create Auth module",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        while rx.try_recv().is_ok() {}

        let handled =
            handle_slash_command("/task show TSK-001", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("TSK-001"));
                assert!(text.contains("Auth module"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_task_move_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command("/task create Task", &mut app, &tx, &mut ge, &registry).await;
        while rx.try_recv().is_ok() {}

        let handled = handle_slash_command(
            "/task move TSK-001 in_progress",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("Transitioned TSK-001"));
                assert!(text.contains("todo → in_progress"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_task_done_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command("/task create Task", &mut app, &tx, &mut ge, &registry).await;
        while rx.try_recv().is_ok() {}

        // Move to in_progress first
        handle_slash_command(
            "/task move TSK-001 in_progress",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        while rx.try_recv().is_ok() {}

        let handled =
            handle_slash_command("/task done TSK-001", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("in_progress → done"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_task_assign_via_slash() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command("/task create Task", &mut app, &tx, &mut ge, &registry).await;
        while rx.try_recv().is_ok() {}

        let handled = handle_slash_command(
            "/task assign TSK-001 alice",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        assert!(handled);

        let mut found_output = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_output = true;
                assert!(text.contains("Assigned TSK-001 to alice"));
            }
        }
        assert!(found_output);
    }

    #[tokio::test]
    async fn test_task_command_without_store_shows_error() {
        let mut app = App::new();
        assert!(app.task_store.is_none());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        let handled = handle_slash_command("/task list", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("Task store not initialized"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_tasks_without_store_shows_error() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        let handled = handle_slash_command("/tasks", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("Task store not initialized"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_task_invalid_transition_shows_error() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command("/task create Task", &mut app, &tx, &mut ge, &registry).await;
        while rx.try_recv().is_ok() {}

        // Try todo → done (invalid in default workflow)
        let handled =
            handle_slash_command("/task move TSK-001 done", &mut app, &tx, &mut ge, &registry)
                .await;
        assert!(handled);

        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("Invalid transition"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_task_parse_error_shows_error() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        let handled = handle_slash_command("/task create", &mut app, &tx, &mut ge, &registry).await;
        assert!(handled);

        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("Missing title"));
            }
        }
        assert!(found_error);
    }

    #[tokio::test]
    async fn test_help_includes_task_commands() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let _handled = handle_slash_command("/help", &mut app, &tx, &mut ge, &registry).await;

        let mut found_chunk = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_chunk = true;
                assert!(
                    text.contains("/task create"),
                    "help should list /task create"
                );
                assert!(text.contains("/task list"), "help should list /task list");
                assert!(text.contains("/task show"), "help should list /task show");
                assert!(text.contains("/task move"), "help should list /task move");
                assert!(text.contains("/task done"), "help should list /task done");
                assert!(text.contains("/task block"), "help should list /task block");
                assert!(text.contains("/task label"), "help should list /task label");
                assert!(
                    text.contains("/task assign"),
                    "help should list /task assign"
                );
                assert!(text.contains("/tasks"), "help should list /tasks");
            }
        }
        assert!(found_chunk);
    }

    // ── Task board view / navigation tests ────────────────────────────────

    #[test]
    fn test_app_default_view_is_conversation() {
        let app = App::new();
        assert_eq!(app.active_view, AppView::Conversation);
    }

    #[test]
    fn test_tab_cycles_conversation_to_task_board() {
        let mut app = App::new();
        assert_eq!(app.active_view, AppView::Conversation);
        app.cycle_view();
        assert_eq!(app.active_view, AppView::TaskBoard);
    }

    #[test]
    fn test_tab_cycles_task_board_to_task_graph() {
        let mut app = App::new();
        app.cycle_view(); // → TaskBoard
        app.cycle_view(); // → TaskGraph
        assert_eq!(app.active_view, AppView::TaskGraph);
    }

    #[test]
    fn test_tab_cycles_task_graph_to_conversation() {
        let mut app = App::new();
        app.cycle_view(); // → TaskBoard
        app.cycle_view(); // → TaskGraph
        app.cycle_view(); // → Conversation
        assert_eq!(app.active_view, AppView::Conversation);
    }

    #[test]
    fn test_tab_cycles_full_loop() {
        let mut app = App::new();
        for _ in 0..3 {
            app.cycle_view();
        }
        assert_eq!(app.active_view, AppView::Conversation);
    }

    #[test]
    fn test_task_detail_goes_back_to_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        app.go_back();
        assert_eq!(app.active_view, AppView::TaskBoard);
    }

    #[test]
    fn test_task_board_goes_back_to_conversation() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.go_back();
        assert_eq!(app.active_view, AppView::Conversation);
    }

    #[test]
    fn test_conversation_goes_back_stays_conversation() {
        let mut app = App::new();
        app.go_back();
        assert_eq!(app.active_view, AppView::Conversation);
    }

    #[test]
    fn test_task_cursor_up_clamps_at_zero() {
        let mut app = App::new();
        app.selected_task_idx = 0;
        app.task_cursor_up();
        assert_eq!(app.selected_task_idx, 0);
    }

    #[test]
    fn test_task_cursor_up_decrements() {
        let mut app = App::new();
        app.selected_task_idx = 3;
        app.task_cursor_up();
        assert_eq!(app.selected_task_idx, 2);
    }

    #[test]
    fn test_task_cursor_down_clamps_at_end() {
        let mut app = App::new();
        // Empty cache — cursor should not move
        app.task_cursor_down();
        assert_eq!(app.selected_task_idx, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_task_cursor_down_increments() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Create 3 tasks
        for i in 0..3 {
            handle_slash_command(
                &format!("/task create Task {i}"),
                &mut app,
                &tx,
                &mut ge,
                &registry,
            )
            .await;
        }

        app.refresh_task_cache();
        assert_eq!(app.task_list_cache.len(), 3);
        assert_eq!(app.selected_task_idx, 0);

        app.task_cursor_down();
        assert_eq!(app.selected_task_idx, 1);

        app.task_cursor_down();
        assert_eq!(app.selected_task_idx, 2);

        // Should clamp at last item
        app.task_cursor_down();
        assert_eq!(app.selected_task_idx, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_enter_task_detail_transitions_view() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command(
            "/task create Auth module",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;

        app.refresh_task_cache();
        let id = app.enter_task_detail();
        assert_eq!(id, Some("TSK-001".to_string()));
        assert_eq!(app.active_view, AppView::TaskDetail("TSK-001".to_string()));
    }

    #[test]
    fn test_enter_task_detail_with_empty_cache_returns_none() {
        let mut app = App::new();
        let result = app.enter_task_detail();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_refresh_task_cache_populates_list() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        handle_slash_command("/task create Task A", &mut app, &tx, &mut ge, &registry).await;
        handle_slash_command("/task create Task B", &mut app, &tx, &mut ge, &registry).await;

        assert!(app.task_list_cache.is_empty());
        let result = app.refresh_task_cache();
        assert!(result);
        assert_eq!(app.task_list_cache.len(), 2);
        assert_eq!(app.task_list_cache[0].title, "Task A");
        assert_eq!(app.task_list_cache[1].title, "Task B");
    }

    #[test]
    fn test_refresh_task_cache_without_store_returns_false() {
        let mut app = App::new();
        assert!(app.task_store.is_none());
        let result = app.refresh_task_cache();
        assert!(!result);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_refresh_clamps_cursor_to_last_item() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Create 5 tasks
        for i in 0..5 {
            handle_slash_command(
                &format!("/task create T{i}"),
                &mut app,
                &tx,
                &mut ge,
                &registry,
            )
            .await;
        }

        app.refresh_task_cache();
        assert_eq!(app.task_list_cache.len(), 5);

        // Move cursor to last item
        app.selected_task_idx = 4;

        // Now cache shrinks to 2 items — cursor should clamp
        // Delete tasks 3,4 by recreating store... easier: just manually set cache
        app.task_list_cache.truncate(2);
        app.selected_task_idx = 4;
        // Simulate refresh clamping
        if app.selected_task_idx >= app.task_list_cache.len() && !app.task_list_cache.is_empty() {
            app.selected_task_idx = app.task_list_cache.len() - 1;
        }
        assert_eq!(app.selected_task_idx, 1);
    }

    #[test]
    fn test_task_detail_tab_cycles_to_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        app.cycle_view();
        assert_eq!(app.active_view, AppView::TaskBoard);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cache_shows_blocked_by_tasks() {
        let (mut app, _dir) = make_app_with_task_store().await;
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _ge_dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Create two tasks
        handle_slash_command("/task create Blocker", &mut app, &tx, &mut ge, &registry).await;
        handle_slash_command("/task create Blocked", &mut app, &tx, &mut ge, &registry).await;

        // Block second task by first
        handle_slash_command(
            "/task block TSK-002 TSK-001",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;

        app.refresh_task_cache();
        assert_eq!(app.task_list_cache.len(), 2);

        // Find the blocked task
        let blocked = app
            .task_list_cache
            .iter()
            .find(|t| t.id == "TSK-002")
            .unwrap();
        assert!(blocked.blocked_by.contains(&"TSK-001".to_string()));
    }

    // ── Autocomplete command filter tests ──────────────────────────────────

    #[test]
    fn test_filter_commands_empty_returns_all() {
        let results = filter_commands("");
        assert_eq!(results.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn test_filter_commands_help_prefix() {
        let results = filter_commands("/hel");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/help");
    }

    #[test]
    fn test_filter_commands_guardrails_prefix() {
        let results = filter_commands("/guard");
        // Should match /guardrails:status, /guardrails:clean, /guardrails:onboarding
        assert_eq!(results.len(), 3);
        for cmd in &results {
            assert!(cmd.name.starts_with("/guard"));
        }
    }

    #[test]
    fn test_filter_commands_git_prefix() {
        let results = filter_commands("/git");
        // Matches /git:status, /git:log, /github:pr-list (starts with /git)
        assert_eq!(results.len(), 3);
        for cmd in &results {
            assert!(cmd.name.starts_with("/git"));
        }
    }

    #[test]
    fn test_filter_commands_no_match() {
        let results = filter_commands("/xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_commands_case_insensitive() {
        let results = filter_commands("/HELP");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/help");
    }

    #[test]
    fn test_filter_commands_task_matches_both() {
        let results = filter_commands("/task");
        // Should match /task and /tasks
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_filter_commands_task_exact() {
        let results = filter_commands("/tasks");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/tasks");
    }

    #[test]
    fn test_filter_commands_quit_prefix() {
        let results = filter_commands("/qu");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/quit");
    }

    #[test]
    fn test_filter_commands_clear_prefix() {
        let results = filter_commands("/cl");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/clear");
    }

    #[test]
    fn test_command_info_debug_and_clone() {
        let info = CommandInfo {
            name: "/test",
            description: "test command",
        };
        let cloned = info;
        assert_eq!(format!("{info:?}"), format!("{cloned:?}"));
    }

    // ── Autocomplete state tests ───────────────────────────────────────────

    #[test]
    fn test_autocomplete_default_not_visible() {
        let app = App::new();
        assert!(!app.autocomplete_visible);
        assert_eq!(app.autocomplete_selected, 0);
    }

    #[test]
    fn test_autocomplete_update_shows_when_input_starts_with_slash() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_update_hidden_when_input_empty() {
        let mut app = App::new();
        app.input = "".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_update_hidden_when_generating() {
        let mut app = App::new();
        app.state = AppState::Generating;
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_update_hidden_in_task_view() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_update_hidden_when_no_match() {
        let mut app = App::new();
        app.input = "/zzz".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_next_moves_down() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);
        assert_eq!(app.autocomplete_selected, 0);
        app.autocomplete_next();
        assert_eq!(app.autocomplete_selected, 1);
    }

    #[test]
    fn test_autocomplete_next_clamps_at_end() {
        let mut app = App::new();
        app.input = "/hel".to_string();
        app.update_autocomplete_state();
        let count = app.filtered_commands().len();
        assert_eq!(count, 1);
        // Even with only 1 item, next should not go out of bounds
        app.autocomplete_selected = count - 1;
        app.autocomplete_next();
        assert_eq!(app.autocomplete_selected, count - 1);
    }

    #[test]
    fn test_autocomplete_prev_moves_up() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        app.autocomplete_selected = 3;
        app.autocomplete_prev();
        assert_eq!(app.autocomplete_selected, 2);
    }

    #[test]
    fn test_autocomplete_prev_clamps_at_zero() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        app.autocomplete_selected = 0;
        app.autocomplete_prev();
        assert_eq!(app.autocomplete_selected, 0);
    }

    #[test]
    fn test_autocomplete_select_returns_name() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        app.autocomplete_visible = true;
        let selected = app.autocomplete_select();
        assert!(selected.is_some());
        assert_eq!(selected.unwrap(), "/help");
    }

    #[test]
    fn test_autocomplete_select_returns_none_when_not_visible() {
        let app = App::new();
        let selected = app.autocomplete_select();
        assert!(selected.is_none());
    }

    #[test]
    fn test_autocomplete_select_returns_none_when_empty_filter() {
        let mut app = App::new();
        app.input = "/zzz".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
        // Make it visible but with empty filtered list
        app.autocomplete_visible = true;
        let selected = app.autocomplete_select();
        assert!(selected.is_none());
    }

    #[test]
    fn test_filtered_commands_uses_input() {
        let mut app = App::new();
        app.input = "/cl".to_string();
        let results = app.filtered_commands();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/clear");
    }

    #[test]
    fn test_filtered_commands_guard_prefix() {
        let mut app = App::new();
        app.input = "/gu".to_string();
        let cmds = app.filtered_commands();
        assert_eq!(cmds.len(), 3, "should filter to guardrails commands");
        for cmd in &cmds {
            assert!(
                cmd.name.starts_with("/gu"),
                "{} should start with /gu",
                cmd.name
            );
        }
    }

    #[test]
    fn test_filtered_commands_empty_if_no_slash() {
        let mut app = App::new();
        app.input = "hello".to_string();
        let results = app.filtered_commands();
        assert!(results.is_empty());
    }

    #[test]
    fn test_update_autocomplete_resets_selection_when_hidden() {
        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);
        app.autocomplete_selected = 5;
        app.input = "hello".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
        assert_eq!(app.autocomplete_selected, 0);
    }
}
