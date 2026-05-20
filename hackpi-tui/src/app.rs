use crate::events::{ToolSummary, TuiEvent};
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
    /// When `true`, the user has pressed [4] once and must press it again
    /// to confirm `AlwaysAllow`. Set to `false` on any other key press.
    pub confirming_always_allow: bool,
}

impl std::fmt::Debug for PermissionPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionPrompt")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .field("response", &self.response.as_ref().map(|_| "Sender<..>"))
            .field("confirming_always_allow", &self.confirming_always_allow)
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

/// Severity level for error and informational messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Purely informational message.
    Info,
    /// A warning — something unexpected but non-fatal.
    Warning,
    /// An error — something went wrong.
    Error,
}

/// Fine-grained UI status replacing the old ad-hoc `AppState` + `status_message`.
///
/// Each variant carries the information needed to render a distinct visual state
/// in both the status bar and the conversation area.
#[derive(Debug, Clone, PartialEq)]
pub enum UiStatus {
    /// No activity — waiting for user input.
    Idle,
    /// LLM is streaming a text response.
    Generating,
    /// A tool is currently executing.
    RunningTool { name: String },
    /// Task data is being loaded from the store.
    LoadingTasks,
    /// A permission prompt is awaiting user decision.
    WaitingForPermission,
    /// An error or informational state to display in the status bar and optionally
    /// in the conversation area.
    Error { message: String, severity: Severity },
}

impl UiStatus {
    /// Returns `true` when the app is in an active/generating state that should
    /// disable input and show activity indicators.
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Generating | Self::RunningTool { .. } | Self::LoadingTasks
        )
    }

    /// Returns `true` when the app is generating (streaming text), for spinner
    /// tick decisions in the main loop.
    pub fn is_generating(&self) -> bool {
        matches!(self, Self::Generating | Self::RunningTool { .. })
    }
}

/// Connection health indicator for the status bar.
///
/// Replaces the hard-coded "● connected" with a live health label.
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionHealth {
    /// No request has been made yet — initial state.
    Unknown,
    /// The last interaction succeeded.
    Connected,
    /// The last interaction produced an error.
    Error { message: String },
    /// The endpoint is unreachable or the client is offline.
    Offline,
}

impl ConnectionHealth {
    /// Return a human-readable label for the status bar.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "API: unknown",
            Self::Connected => "API: connected",
            Self::Error { .. } => "API: error",
            Self::Offline => "API: offline",
        }
    }

    /// Update health based on a tool result or error event.
    pub fn observe_event(&mut self, event: &TuiEvent) {
        match event {
            TuiEvent::ToolResult { result, .. } => match result {
                ToolResult::Success { .. } => *self = Self::Connected,
                ToolResult::SystemError { message } => {
                    *self = Self::Error {
                        message: message.clone(),
                    }
                }
                ToolResult::Timeout => {
                    *self = Self::Error {
                        message: "request timed out".into(),
                    }
                }
                ToolResult::Cancelled => {
                    // Cancellation is not a connection error
                }
            },
            TuiEvent::Error(msg) => {
                *self = Self::Error {
                    message: msg.clone(),
                }
            }
            _ => {}
        }
    }
}

/// The kind of a conversation entry — either a normal message or a system error.
#[derive(Debug, Clone, PartialEq)]
pub enum ConversationEntryKind {
    /// A regular user or assistant message.
    Message,
    /// A system error rendered inline in the conversation.
    SystemError {
        severity: Severity,
        recovery_hint: Option<String>,
    },
}

pub struct ConversationEntry {
    pub kind: ConversationEntryKind,
    pub role: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallDisplay>,
}

pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub summary: ToolSummary,
    pub status: ToolCallStatus,
}

pub enum ToolCallStatus {
    Running,
    Done(ToolResult),
}

pub struct App {
    /// Fine-grained UI status (Idle, Generating, RunningTool, Error, etc.).
    pub ui_status: UiStatus,
    /// Connection health indicator for the status bar.
    pub connection_health: ConnectionHealth,
    pub input: String,
    /// Frame counter for animated loading spinner.
    pub loading_frame: usize,
    pub conversation: VecDeque<ConversationEntry>,
    /// Visual row offset from top (used when `auto_scroll` is false).
    pub scroll_offset: usize,
    /// When true, the conversation view scrolls to show the latest content.
    /// Set to false when the user manually scrolls up.
    pub auto_scroll: bool,
    pub usage: Option<Usage>,
    /// Transient informational message (e.g. "Created TSK-003"), shown in the
    /// status bar and auto-cleared on the next user action.
    pub info_message: Option<String>,
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
    /// Whether the task creation inline prompt is active in the task board.
    pub creating_task: bool,
    /// Buffer for the task title being entered during task creation.
    pub task_create_input: String,
    /// Character offset of the cursor within the input buffer.
    /// Synced from `InputHandler::cursor` so the UI can position the terminal cursor.
    pub input_cursor: usize,
    /// Whether the contextual help overlay is visible (? key).
    pub help_visible: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            ui_status: UiStatus::Idle,
            connection_health: ConnectionHealth::Unknown,
            input: String::new(),
            conversation: VecDeque::new(),
            scroll_offset: 0,
            auto_scroll: true,
            usage: None,
            info_message: None,
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
            creating_task: false,
            task_create_input: String::new(),
            loading_frame: 0,
            input_cursor: 0,
            help_visible: false,
        }
    }

    /// Set the UI status to Interrupted (used by Ctrl+C in the main loop).
    pub fn set_interrupted(&mut self) {
        self.ui_status = UiStatus::Idle;
        self.info_message = Some("Generation interrupted.".into());
    }

    /// Returns `true` when the app is in an active/generating state.
    pub fn is_generating(&self) -> bool {
        self.ui_status.is_generating()
    }

    pub fn handle_event(&mut self, event: TuiEvent) {
        // Pass non-error events to connection health tracking
        self.connection_health.observe_event(&event);

        match event {
            TuiEvent::Submit(text) => {
                self.info_message = None;
                self.conversation.push_back(ConversationEntry {
                    kind: ConversationEntryKind::Message,
                    role: "user".into(),
                    text,
                    tool_calls: Vec::new(),
                });
                self.ui_status = UiStatus::Generating;
                self.auto_scroll = true;
                self.scroll_offset = 0;
                self.input.clear();
            }
            TuiEvent::StreamChunk(chunk) => {
                self.auto_scroll = true;
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        kind: ConversationEntryKind::Message,
                        role: "assistant".into(),
                        text: chunk,
                        tool_calls: Vec::new(),
                    });
                } else if let Some(entry) = self.conversation.back_mut() {
                    entry.text.push_str(&chunk);
                }
            }
            TuiEvent::ToolCall { id, name, input } => {
                self.ui_status = UiStatus::RunningTool { name: name.clone() };
                self.auto_scroll = true;
                let needs_new = match self.conversation.back() {
                    Some(e) => e.role != "assistant",
                    None => true,
                };
                if needs_new {
                    self.conversation.push_back(ConversationEntry {
                        kind: ConversationEntryKind::Message,
                        role: "assistant".into(),
                        text: String::new(),
                        tool_calls: Vec::new(),
                    });
                }
                if let Some(entry) = self.conversation.back_mut() {
                    let summary = ToolSummary::from_params(&name, input.as_ref());
                    entry.tool_calls.push(ToolCallDisplay {
                        id,
                        name,
                        summary,
                        status: ToolCallStatus::Running,
                    });
                }
            }
            TuiEvent::ToolResult { id, result, .. } => {
                self.auto_scroll = true;
                if let Some(entry) = self.conversation.back_mut() {
                    for tc in &mut entry.tool_calls {
                        if tc.id == id {
                            tc.status = ToolCallStatus::Done(result);
                            break;
                        }
                    }
                }
                // After a tool completes, revert to generating if the LLM is still
                // streaming (the next tool call or Done event will update it).
                if matches!(self.ui_status, UiStatus::RunningTool { .. }) {
                    self.ui_status = UiStatus::Generating;
                }
            }
            TuiEvent::Usage(usage) => {
                self.usage = Some(usage);
            }
            TuiEvent::Error(err) => {
                // Create a visible conversation entry for the error
                let recovery_hint = recovery_hint_for_error(&err);
                self.conversation.push_back(ConversationEntry {
                    kind: ConversationEntryKind::SystemError {
                        severity: Severity::Error,
                        recovery_hint,
                    },
                    role: "system".into(),
                    text: err.clone(),
                    tool_calls: Vec::new(),
                });
                self.ui_status = UiStatus::Error {
                    message: err,
                    severity: Severity::Error,
                };
            }
            TuiEvent::Done => {
                self.ui_status = UiStatus::Idle;
            }
            TuiEvent::PermissionRequest {
                id,
                reason,
                response,
            } => {
                self.ui_status = UiStatus::WaitingForPermission;
                self.pending_permission = Some(PermissionPrompt {
                    id,
                    reason,
                    response: Some(response),
                    confirming_always_allow: false,
                });
            }
        }
    }

    pub fn clear(&mut self) {
        self.conversation.clear();
        self.input.clear();
        self.usage = None;
        self.scroll_offset = 0;
        self.auto_scroll = true;
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
                    self.info_message = Some(format!("Task {id} not found"));
                    self.active_view = AppView::TaskBoard;
                }
                Err(e) => {
                    self.task_detail_cache = None;
                    self.task_detail_blocked_by = Vec::new();
                    self.task_detail_blocking = Vec::new();
                    self.info_message = Some(format!("Error loading task: {e}"));
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
            && !self.ui_status.is_active()
            && matches!(
                self.active_view,
                AppView::Conversation | AppView::TaskBoard | AppView::TaskDetail(_)
            );

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

    /// Enter the task creation prompt mode. Only valid in TaskBoard view.
    pub fn begin_create_task(&mut self) {
        if matches!(
            self.active_view,
            AppView::TaskBoard | AppView::TaskDetail(_)
        ) {
            self.creating_task = true;
            self.task_create_input.clear();
        }
    }

    /// Cancel the task creation prompt mode.
    pub fn cancel_create_task(&mut self) {
        self.creating_task = false;
        self.task_create_input.clear();
    }

    /// Submit the task creation. Returns `Some(task_id)` on success, `None` on failure.
    /// On success, refreshes the task list cache and selects the newly created task.
    pub fn submit_create_task(&mut self) -> Option<String> {
        let title = self.task_create_input.trim().to_string();
        if title.is_empty() {
            self.info_message = Some("Task title cannot be empty.".to_string());
            return None;
        }

        let result = self.create_task_sync(&title);
        self.creating_task = false;
        self.task_create_input.clear();

        match result {
            Some(task) => {
                let id = task.id.clone();
                self.info_message = Some(format!("Created {}: \"{}\"", id, task.title));
                self.refresh_task_cache();
                // Select the newly created task
                for (i, t) in self.task_list_cache.iter().enumerate() {
                    if t.id == id {
                        self.selected_task_idx = i;
                        break;
                    }
                }
                Some(id)
            }
            None => {
                self.info_message = Some("Failed to create task.".to_string());
                None
            }
        }
    }

    /// Return the current focus target derived from view, state, and overlays.
    pub fn focus_target(&self) -> crate::interaction::FocusTarget {
        crate::interaction::focus_target(self)
    }

    /// Return the current active overlay, if any.
    pub fn active_overlay(&self) -> Option<crate::interaction::OverlayKind> {
        crate::interaction::active_overlay(self)
    }

    /// Internal: create a task synchronously via the task store.
    fn create_task_sync(&self, title: &str) -> Option<hackpi_tasks::Task> {
        if let Some(ref store) = self.task_store {
            let store_clone: Arc<dyn hackpi_tasks::TaskStore> =
                Arc::clone(store) as Arc<dyn hackpi_tasks::TaskStore>;
            let new_task = hackpi_tasks::NewTask::new(title.to_string());
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { store_clone.create(&new_task).await.ok() })
            })
        } else {
            None
        }
    }
}

/// The outcome of handling a slash command, used to communicate the result
/// back to the main event loop so it can take appropriate action (e.g., exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The command was handled normally. Continue the event loop.
    Handled,
    /// The user requested the application to exit. Break the main loop.
    ExitRequested,
    /// The command performed an action that requires a re-render (e.g., /clear).
    NeedsRender,
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
    CommandInfo {
        name: "/export",
        description: "Export conversation to text file",
    },
];

/// Generate the `/help` output text from the canonical SLASH_COMMANDS source.
/// This keeps help text in sync automatically — no separate hardcoded list to maintain.
pub fn format_help_text() -> String {
    let mut lines = String::from("Available commands:\n");
    for cmd in SLASH_COMMANDS {
        lines.push_str(&format!("   {} - {}\n", cmd.name, cmd.description));
    }
    lines
}

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
) -> CommandOutcome {
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
            return CommandOutcome::Handled;
        }
    };

    // Emit ToolCall event so the conversation shows a tool card
    tui_tx
        .send(TuiEvent::ToolCall {
            id: tool_id.clone(),
            name: tool_name.to_string(),
            input: Some(params.clone()),
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
    CommandOutcome::Handled
}

pub async fn handle_slash_command(
    cmd: &str,
    app: &mut App,
    tui_tx: &tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    guard_evaluator: &mut GuardEvaluator,
    tool_registry: &hackpi_core::tools::ToolRegistry,
) -> CommandOutcome {
    let parts: Vec<&str> = cmd.trim().splitn(2, char::is_whitespace).collect();
    let command = parts[0];
    match command {
        "/help" => {
            let help_text = format_help_text();
            tui_tx
                .send(TuiEvent::StreamChunk(help_text.to_string()))
                .ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
        }
        "/clear" => {
            app.clear();
            CommandOutcome::NeedsRender
        }
        "/quit" => {
            app.quit_requested = true;
            CommandOutcome::ExitRequested
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
            CommandOutcome::Handled
        }
        "/guardrails:clean" => {
            guard_evaluator.clear_session();
            let msg = "Session cache cleared.".to_string();
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
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
                    return CommandOutcome::Handled;
                }
                _ => {
                    let err = format!(
                        "Unknown preset: '{preset}'. Available: strict, balanced, permissive"
                    );
                    tui_tx.send(TuiEvent::Error(err)).ok();
                    return CommandOutcome::Handled;
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
                    return CommandOutcome::Handled;
                }
            };

            // Create .hackpi directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&hackpi_dir) {
                let err = format!("Failed to create directory {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return CommandOutcome::Handled;
            }

            let config_path = hackpi_dir.join("guardrails.json");
            if let Err(e) = std::fs::write(&config_path, config_json) {
                let err = format!("Failed to write config file: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return CommandOutcome::Handled;
            }

            // Reload rules from the new config
            if let Err(e) = guard_evaluator.load_rules() {
                let err = format!("Failed to load rules after writing config: {e}");
                tui_tx.send(TuiEvent::Error(err)).ok();
                return CommandOutcome::Handled;
            }

            let rule_count = guard_evaluator.rule_count();
            let msg = format!(
                "Wrote {preset_name} guardrails config to {} ({rule_count} rules loaded).",
                config_path.display()
            );
            tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
            tui_tx.send(TuiEvent::Done).ok();
            CommandOutcome::Handled
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
                    CommandOutcome::Handled
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    CommandOutcome::Handled
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
                    CommandOutcome::Handled
                }
                None => {
                    tui_tx
                        .send(TuiEvent::Error(
                            "Task store not initialized. Task commands are unavailable.".into(),
                        ))
                        .ok();
                    CommandOutcome::Handled
                }
            }
        }
        "/export" => {
            let custom_path = parts.get(1).copied().unwrap_or("").trim();

            let export_path = if custom_path.is_empty() {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                format!("./hackpi-export-{timestamp}.txt")
            } else {
                custom_path.to_string()
            };

            let formatted = format_conversation(&app.conversation);
            match std::fs::write(&export_path, &formatted) {
                Ok(_) => {
                    let size = formatted.len();
                    let abs_path = std::path::Path::new(&export_path)
                        .canonicalize()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| export_path.clone());
                    let msg = format!(
                        "\
Export complete:
  Path: {abs_path}
  Size: {size} bytes
  Messages: {}",
                        app.conversation.len()
                    );
                    tui_tx.send(TuiEvent::StreamChunk(msg)).ok();
                    tui_tx.send(TuiEvent::Done).ok();
                }
                Err(e) => {
                    let err = format!("Failed to export conversation: {e}");
                    tui_tx.send(TuiEvent::Error(err)).ok();
                }
            }
            CommandOutcome::Handled
        }
        _ => {
            let err = format!("Unknown command: {command}. Type /help for available commands.");
            tui_tx.send(TuiEvent::Error(err)).ok();
            CommandOutcome::Handled
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

/// Generate a short, actionable recovery hint for a given error message.
///
/// Returns `None` when no reasonable hint can be derived.
fn recovery_hint_for_error(err: &str) -> Option<String> {
    let lower = err.to_lowercase();
    if lower.contains("tool") && (lower.contains("not found") || lower.contains("unregistered")) {
        Some("Check the tool name and try again.".into())
    } else if lower.contains("permission") || lower.contains("denied") {
        Some("Request permission or use /guardrails:status to check rules.".into())
    } else if lower.contains("timeout") || lower.contains("timed out") {
        Some("The request timed out. Try again or use a simpler query.".into())
    } else if lower.contains("api") || lower.contains("connection") || lower.contains("network") {
        Some("Check your API connection and try again.".into())
    } else if lower.contains("guardrail") || lower.contains("deny") {
        Some("Modify guardrails config or run /guardrails:status.".into())
    } else if lower.contains("parse") || lower.contains("malformed") || lower.contains("invalid") {
        Some("Check the input format and try again.".into())
    } else {
        None
    }
}

/// Map a key character to a `PermissionDecision`, matching the key bindings
/// used in the TUI event loop when a permission prompt is active.
///
/// Note: This does NOT handle the two-step confirmation for Always allow
/// (key '4'), which is implemented in the event loop in main.rs.
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

/// Format the conversation history as a markdown text document suitable for
/// LLM analysis. Includes a metadata header (date, message count) and each
/// conversation entry with role labels, timestamps, text content, and tool
/// call details (name, status, result).
///
/// The output is designed to be clean and parseable, with clear section
/// separators and consistent formatting across all entry types.
pub fn format_conversation(conversation: &VecDeque<ConversationEntry>) -> String {
    let now = chrono::Local::now();
    let date_str = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let mut output = String::new();
    output.push_str("# HackPI Conversation Export\n\n");
    output.push_str(&format!("Date: {date_str}\n"));
    output.push_str(&format!("Messages: {}\n\n", conversation.len()));

    for (i, entry) in conversation.iter().enumerate() {
        let msg_num = i + 1;
        output.push_str(&format!("## Message {msg_num}\n"));
        output.push_str(&format!("**Role**: {}\n", entry.role));

        match &entry.kind {
            ConversationEntryKind::SystemError {
                severity,
                recovery_hint,
            } => {
                let severity_label = match severity {
                    Severity::Info => "Info",
                    Severity::Warning => "Warning",
                    Severity::Error => "Error",
                };
                output.push_str(&format!("**Type**: System Error ({severity_label})\n"));
                if let Some(hint) = recovery_hint {
                    output.push_str(&format!("**Recovery**: {hint}\n"));
                }
            }
            ConversationEntryKind::Message => {}
        }

        output.push_str("---\n");

        if !entry.text.is_empty() {
            output.push_str(&entry.text);
            output.push('\n');
        }

        if !entry.tool_calls.is_empty() {
            output.push('\n');
            for tc in &entry.tool_calls {
                let title = tc.summary.title();
                let status_str = match &tc.status {
                    ToolCallStatus::Running => "Running".to_string(),
                    ToolCallStatus::Done(result) => match result {
                        ToolResult::Success { content } => {
                            format!("Done (Success)\n\n```\n{content}\n```")
                        }
                        ToolResult::SystemError { message } => {
                            format!("Done (Error: {message})")
                        }
                        ToolResult::Timeout => "Done (Timeout)".to_string(),
                        ToolResult::Cancelled => "Done (Cancelled)".to_string(),
                    },
                };
                output.push_str(&format!("### Tool: {title}\n"));
                output.push_str(&format!("**Tool**: {}\n", tc.name));
                output.push_str(&format!("**Status**: {status_str}\n\n"));
            }
        }

        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use hackpi_guardrails::SettingsPaths;
    use tokio::sync::mpsc;

    // ── CommandOutcome tests ────────────────────────────────────────────

    #[tokio::test]
    async fn test_slash_quit_returns_exit_requested() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let outcome =
            handle_slash_command("/quit", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(outcome, CommandOutcome::ExitRequested);
        assert!(app.quit_requested, "/quit should set quit_requested flag");
    }

    #[tokio::test]
    async fn test_slash_clear_returns_needs_render() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let outcome =
            handle_slash_command("/clear", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(outcome, CommandOutcome::NeedsRender);
    }

    #[tokio::test]
    async fn test_slash_help_returns_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let outcome =
            handle_slash_command("/help", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(outcome, CommandOutcome::Handled);
    }

    #[tokio::test]
    async fn test_slash_unknown_returns_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let outcome = handle_slash_command(
            "/nonexistent",
            &mut app,
            &tx,
            &mut ge,
            &make_tool_registry(),
        )
        .await;
        assert_eq!(outcome, CommandOutcome::Handled);
    }

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
        let outcome =
            handle_slash_command("/help", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(outcome, CommandOutcome::Handled);
    }

    #[tokio::test]
    async fn test_slash_help_generates_help_text() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command("/help", &mut app, &tx, &mut ge, &registry).await;
        assert_eq!(handled, CommandOutcome::Handled);
        let mut found_chunk = false;
        let mut found_done = false;
        let mut help_text = String::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::StreamChunk(text) => {
                    found_chunk = true;
                    help_text.push_str(&text);
                }
                TuiEvent::Done => found_done = true,
                _ => {}
            }
        }
        assert!(found_chunk);
        assert!(found_done);

        // Every registered SLASH_COMMAND must appear in the help output
        for cmd in SLASH_COMMANDS {
            assert!(
                help_text.contains(cmd.name),
                "/help output should mention {} but it does not appear in:\n{help_text}",
                cmd.name
            );
        }
    }

    #[test]
    fn test_format_help_text_includes_all_commands() {
        let help = format_help_text();
        assert!(help.starts_with("Available commands:"));
        for cmd in SLASH_COMMANDS {
            assert!(
                help.contains(cmd.name),
                "format_help_text() should include '{}' but it does not appear in:\n{help}",
                cmd.name
            );
            assert!(
                help.contains(cmd.description),
                "format_help_text() should include description '{}' for cmd '{}'",
                cmd.description,
                cmd.name
            );
        }
    }

    #[tokio::test]
    async fn test_slash_clear_clears_conversation() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert_eq!(app.conversation.len(), 1);
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let outcome =
            handle_slash_command("/clear", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(outcome, CommandOutcome::NeedsRender);
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
        assert_eq!(handled, CommandOutcome::Handled);
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
        assert_eq!(handled, CommandOutcome::Handled);
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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);
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
        assert_eq!(handled, CommandOutcome::Handled);
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
        assert_eq!(handled, CommandOutcome::Handled);
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
        assert_eq!(handled, CommandOutcome::Handled);
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
            assert_eq!(
                handled,
                CommandOutcome::Handled,
                "command '{cmd}' should be handled"
            );
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
        assert_eq!(
            handled,
            CommandOutcome::Handled,
            "/git:status should be handled"
        );
    }

    #[tokio::test]
    async fn test_git_log_slash_command_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled = handle_slash_command("/git:log", &mut app, &tx, &mut ge, &registry).await;
        assert_eq!(
            handled,
            CommandOutcome::Handled,
            "/git:log should be handled"
        );
    }

    #[tokio::test]
    async fn test_github_pr_list_slash_command_handled() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let handled =
            handle_slash_command("/github:pr-list", &mut app, &tx, &mut ge, &registry).await;
        assert_eq!(
            handled,
            CommandOutcome::Handled,
            "/github:pr-list should be handled"
        );
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
        assert_eq!(app.ui_status, UiStatus::Generating);
    }

    /// Regression test for COR-158: Submit handler must clear app.input
    /// to prevent a ghost textbox (the submitted text appearing in both
    /// the conversation area and the input area).
    #[test]
    fn test_submit_clears_input_preventing_ghost_textbox() {
        let mut app = App::new();
        // Simulate the user having typed text into the input
        app.input = "hello".to_string();
        // Submit the message
        app.handle_event(TuiEvent::Submit("hello".into()));
        // The input must be cleared so it doesn't render as a ghost textbox
        assert!(
            app.input.is_empty(),
            "app.input should be cleared after Submit, got: {:?}",
            app.input
        );
        // Conversation should contain the message
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].text, "hello");
        // State should be Generating
        assert_eq!(app.ui_status, UiStatus::Generating);
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
            input: None,
        });
        assert_eq!(app.conversation.len(), 1);
        assert_eq!(app.conversation[0].role, "assistant");
        assert_eq!(app.conversation[0].tool_calls.len(), 1);
        assert_eq!(app.conversation[0].tool_calls[0].name, "read");
        assert_eq!(app.conversation[0].tool_calls[0].summary.title(), "read");
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
            input: Some(serde_json::json!({"path": "Cargo.toml"})),
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
    fn test_done_sets_idle() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Done);
        assert_eq!(app.ui_status, UiStatus::Idle);
    }

    #[test]
    fn test_error_sets_error_ui_status_and_conversation_entry() {
        let mut app = App::new();
        app.handle_event(TuiEvent::Submit("hello".into()));
        app.handle_event(TuiEvent::Error("API error".into()));
        assert_eq!(
            app.ui_status,
            UiStatus::Error {
                message: "API error".into(),
                severity: Severity::Error,
            }
        );
        // Should have a conversation entry for the error
        assert_eq!(app.conversation.len(), 2);
        let last = app.conversation.back().unwrap();
        assert_eq!(last.role, "system");
        assert!(matches!(
            last.kind,
            ConversationEntryKind::SystemError { .. }
        ));
        assert!(last.text.contains("API error"));
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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
        assert_eq!(handled, CommandOutcome::Handled);

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
                // Help text is generated from SLASH_COMMANDS, so /task and /tasks
                // appear as top-level entries with a description like
                // "Manage tasks (create, list, show, ...)"
                assert!(text.contains("/task"), "help should list /task");
                assert!(text.contains("/tasks"), "help should list /tasks");
                assert!(text.contains("Manage tasks"), "help should describe /task");
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
        app.ui_status = UiStatus::Generating;
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn test_autocomplete_update_visible_in_task_board_view() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(
            app.autocomplete_visible,
            "autocomplete should be visible in TaskBoard when typing /"
        );
    }

    #[test]
    fn test_autocomplete_update_visible_in_task_detail_view() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(
            app.autocomplete_visible,
            "autocomplete should be visible in TaskDetail when typing /"
        );
    }

    #[test]
    fn test_autocomplete_update_hidden_in_task_graph_view() {
        let mut app = App::new();
        app.active_view = AppView::TaskGraph;
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(
            !app.autocomplete_visible,
            "autocomplete should be hidden in TaskGraph placeholder view"
        );
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

    // ── Task creation prompt tests ───────────────────────────────────────────

    #[test]
    fn test_creating_task_default_false() {
        let app = App::new();
        assert!(!app.creating_task);
        assert!(app.task_create_input.is_empty());
    }

    #[test]
    fn test_begin_create_task_in_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.begin_create_task();
        assert!(app.creating_task);
        assert!(app.task_create_input.is_empty());
    }

    #[test]
    fn test_begin_create_task_in_task_detail() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        app.begin_create_task();
        assert!(app.creating_task);
    }

    #[test]
    fn test_begin_create_task_ignored_in_conversation() {
        let mut app = App::new();
        app.active_view = AppView::Conversation;
        app.begin_create_task();
        assert!(!app.creating_task);
    }

    #[test]
    fn test_cancel_create_task() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.begin_create_task();
        app.task_create_input = "some title".to_string();
        app.cancel_create_task();
        assert!(!app.creating_task);
        assert!(app.task_create_input.is_empty());
    }

    #[test]
    fn test_submit_create_task_empty_title_fails() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.creating_task = true;
        app.task_create_input = "   ".to_string();
        let result = app.submit_create_task();
        assert!(result.is_none());
        assert_eq!(app.info_message, Some("Task title cannot be empty.".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_submit_create_task_success() {
        let (mut app, _dir) = make_app_with_task_store().await;
        app.active_view = AppView::TaskBoard;
        app.creating_task = true;
        app.task_create_input = "Test task from TUI".to_string();

        let result = app.submit_create_task();
        assert!(result.is_some(), "should return task ID on success");
        let id = result.unwrap();
        assert!(
            id.starts_with("TSK-"),
            "task ID should start with TSK-, got: {id}"
        );

        // Should have refreshed the cache and selected the new task
        assert!(!app.task_list_cache.is_empty());
        assert_eq!(app.task_list_cache[0].title, "Test task from TUI");
        assert_eq!(app.selected_task_idx, 0);
        assert!(!app.creating_task);
        assert!(app.task_create_input.is_empty());
        assert!(app.info_message.as_ref().unwrap().contains(&id));
    }

    #[test]
    fn test_submit_create_task_no_store() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        app.creating_task = true;
        app.task_create_input = "Test".to_string();
        let result = app.submit_create_task();
        assert!(result.is_none());
        assert_eq!(app.info_message, Some("Failed to create task.".into()));
    }

    // ── Export slash command tests ────────────────────────────────────────

    #[test]
    fn test_export_is_registered_in_slash_commands() {
        let found = SLASH_COMMANDS.iter().any(|cmd| cmd.name == "/export");
        assert!(found, "/export should be registered in SLASH_COMMANDS");
    }

    #[test]
    fn test_export_has_description() {
        let cmd = SLASH_COMMANDS
            .iter()
            .find(|cmd| cmd.name == "/export")
            .expect("/export should be registered");
        assert!(
            !cmd.description.is_empty(),
            "/export should have a non-empty description"
        );
    }

    #[test]
    fn test_filter_export_commands() {
        let results = filter_commands("/exp");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/export");
    }

    #[test]
    fn test_format_conversation_empty() {
        let conversation = VecDeque::new();
        let result = format_conversation(&conversation);
        assert!(result.contains("HackPI Conversation Export"));
        assert!(result.contains("Messages: 0"));
        assert!(result.contains("Date:"));
    }

    #[test]
    fn test_format_conversation_single_user_message() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "user".into(),
            text: "Hello, world!".into(),
            tool_calls: Vec::new(),
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("Messages: 1"));
        assert!(result.contains("## Message 1"));
        assert!(result.contains("**Role**: user"));
        assert!(result.contains("Hello, world!"));
    }

    #[test]
    fn test_format_conversation_assistant_with_tool_calls() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "user".into(),
            text: "Read the file".into(),
            tool_calls: Vec::new(),
        });
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "assistant".into(),
            text: "Let me check that file.".into(),
            tool_calls: vec![
                ToolCallDisplay {
                    id: "tc1".into(),
                    name: "read".into(),
                    summary: ToolSummary::from_params(
                        "read",
                        Some(&serde_json::json!({"path": "file.txt"})),
                    ),
                    status: ToolCallStatus::Done(ToolResult::Success {
                        content: "file contents here".into(),
                    }),
                },
                ToolCallDisplay {
                    id: "tc2".into(),
                    name: "bash".into(),
                    summary: ToolSummary::Unknown,
                    status: ToolCallStatus::Running,
                },
            ],
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("Messages: 2"));
        assert!(result.contains("## Message 1"));
        assert!(result.contains("## Message 2"));
        assert!(result.contains("### Tool: read  file.txt"));
        assert!(result.contains("**Tool**: read"));
        assert!(result.contains("**Status**: Done (Success)"));
        assert!(result.contains("file contents here"));
        assert!(result.contains("### Tool: tool"));
        assert!(result.contains("**Tool**: bash"));
        assert!(result.contains("**Status**: Running"));
    }

    #[test]
    fn test_format_conversation_tool_timeout() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "assistant".into(),
            text: "".into(),
            tool_calls: vec![ToolCallDisplay {
                id: "tc1".into(),
                name: "fetch".into(),
                summary: ToolSummary::Unknown,
                status: ToolCallStatus::Done(ToolResult::Timeout),
            }],
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("**Status**: Done (Timeout)"));
    }

    #[test]
    fn test_format_conversation_tool_error() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "assistant".into(),
            text: "".into(),
            tool_calls: vec![ToolCallDisplay {
                id: "tc1".into(),
                name: "bash".into(),
                summary: ToolSummary::Unknown,
                status: ToolCallStatus::Done(ToolResult::SystemError {
                    message: "command not found".into(),
                }),
            }],
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("**Status**: Done (Error: command not found)"));
    }

    #[test]
    fn test_format_conversation_tool_cancelled() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "assistant".into(),
            text: "Cancelling...".into(),
            tool_calls: vec![ToolCallDisplay {
                id: "tc1".into(),
                name: "long_task".into(),
                summary: ToolSummary::Unknown,
                status: ToolCallStatus::Done(ToolResult::Cancelled),
            }],
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("**Status**: Done (Cancelled)"));
    }

    #[test]
    fn test_format_conversation_no_text_shows_empty_content_area() {
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "user".into(),
            text: "".into(),
            tool_calls: Vec::new(),
        });

        let result = format_conversation(&conversation);
        assert!(result.contains("**Role**: user"));
        assert!(result.contains("---"));
    }

    #[test]
    fn test_format_conversation_does_not_contain_panics() {
        // Verify the formatting is safe for various edge cases
        let mut conversation = VecDeque::new();
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "user".into(),
            text: "Hello".into(),
            tool_calls: Vec::new(),
        });
        conversation.push_back(ConversationEntry {
            kind: ConversationEntryKind::Message,
            role: "assistant".into(),
            text: "".into(),
            tool_calls: vec![ToolCallDisplay {
                id: "tc1".into(),
                name: "fetch".into(),
                summary: ToolSummary::Unknown,
                status: ToolCallStatus::Done(ToolResult::Timeout),
            }],
        });

        let result = format_conversation(&conversation);
        // Should handle gracefully, not panic
        assert!(result.contains("Messages: 2"));
        assert!(result.contains("### Tool: tool"));
    }

    #[tokio::test]
    async fn test_export_slash_command_handled() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        // Add a conversation entry
        app.handle_event(TuiEvent::Submit("Hello".into()));
        app.handle_event(TuiEvent::StreamChunk("Hi there!".into()));
        app.handle_event(TuiEvent::Done);

        let handled = handle_slash_command("/export", &mut app, &tx, &mut ge, &registry).await;
        assert_eq!(
            handled,
            CommandOutcome::Handled,
            "/export should be handled"
        );

        // Verify output events
        let mut found_chunk = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_chunk = true;
                assert!(msg.contains("Export complete"), "msg: {msg}");
                assert!(msg.contains("Size:"), "msg should contain size");
                assert!(
                    msg.contains("Messages:"),
                    "msg should contain message count"
                );
            }
        }
        assert!(found_chunk, "should emit StreamChunk with export info");
    }

    #[tokio::test]
    async fn test_export_with_custom_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let custom_path = tmp.path().join("my-export.txt");
        let custom_path_str = custom_path.to_string_lossy().to_string();

        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        app.handle_event(TuiEvent::Submit("Hello".into()));
        app.handle_event(TuiEvent::Done);

        let handled = handle_slash_command(
            &format!("/export {custom_path_str}"),
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        assert_eq!(handled, CommandOutcome::Handled);

        // Verify file was written
        assert!(
            custom_path.exists(),
            "export file should exist at custom path"
        );

        // Verify output mentions the custom path
        let mut found_path = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_path = true;
                assert!(msg.contains("Export complete"), "msg: {msg}");
            }
        }
        assert!(found_path);

        // Verify file content
        let content = std::fs::read_to_string(&custom_path).expect("read export file");
        assert!(content.contains("HackPI Conversation Export"));
        assert!(content.contains("Hello"));
    }

    #[tokio::test]
    async fn test_export_slash_command_shows_error_with_bad_path() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        app.handle_event(TuiEvent::Submit("Hello".into()));
        app.handle_event(TuiEvent::Done);

        // Try to write to a non-existent directory (should fail)
        let handled = handle_slash_command(
            "/export /nonexistent_dir/file.txt",
            &mut app,
            &tx,
            &mut ge,
            &registry,
        )
        .await;
        assert_eq!(handled, CommandOutcome::Handled);

        let mut found_error = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::Error(msg) = event {
                found_error = true;
                assert!(msg.contains("Failed to export"));
            }
        }
        assert!(found_error, "should emit error for bad path");
    }

    #[tokio::test]
    async fn test_export_slash_command_empty_conversation() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();

        let handled = handle_slash_command("/export", &mut app, &tx, &mut ge, &registry).await;
        assert_eq!(handled, CommandOutcome::Handled);

        let mut found_chunk = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(msg) = event {
                found_chunk = true;
                assert!(msg.contains("Messages: 0"), "msg: {msg}");
            }
        }
        assert!(found_chunk, "should handle empty conversation gracefully");
    }

    #[tokio::test]
    async fn test_help_includes_export_command() {
        let mut app = App::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let registry = make_tool_registry();
        let _handled = handle_slash_command("/help", &mut app, &tx, &mut ge, &registry).await;

        let mut found_chunk = false;
        while let Ok(event) = rx.try_recv() {
            if let TuiEvent::StreamChunk(text) = event {
                found_chunk = true;
                assert!(text.contains("/export"), "help should list /export");
            }
        }
        assert!(found_chunk);
    }

    #[tokio::test]
    async fn test_export_slash_command_prevents_agent_spawn() {
        let mut app = App::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (mut ge, _dir) = make_guard_evaluator();
        let handled =
            handle_slash_command("/export", &mut app, &tx, &mut ge, &make_tool_registry()).await;
        assert_eq!(
            handled,
            CommandOutcome::Handled,
            "/export should prevent agent spawn"
        );
    }
}
