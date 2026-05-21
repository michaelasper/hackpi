use std::collections::VecDeque;
use std::sync::Arc;

use crate::events::TuiEvent;
use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;

use super::conversation::{ConversationEntry, DiagnosticsEntry};
use super::permissions::PermissionPrompt;

/// Active view in the TUI.
#[derive(Debug, Clone, PartialEq)]
pub enum AppView {
    Conversation,
    TaskBoard,
    TaskDetail(String),
    /// Placeholder for a future graph view.
    TaskGraph,
    /// Diagnostics log view showing protocol-level diagnostic messages
    /// (SSE parse failures, stream warnings, etc.) separate from the
    /// conversation viewport.
    Diagnostics,
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

pub struct App {
    /// Fine-grained UI status (Idle, Generating, RunningTool, Error, etc.).
    pub ui_status: UiStatus,
    /// Connection health indicator for the status bar.
    pub connection_health: ConnectionHealth,
    pub input: String,
    /// Frame counter for animated loading spinner.
    pub loading_frame: usize,
    pub conversation: VecDeque<ConversationEntry>,
    /// Protocol-level diagnostics stored separately from the conversation
    /// viewport (SSE parse failures, stream truncation warnings, etc.).
    pub diagnostics: VecDeque<DiagnosticsEntry>,
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
