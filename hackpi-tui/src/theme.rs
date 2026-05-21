use ratatui::style::{Color, Modifier, Style};

/// The complete set of semantic style slots for the hackpi TUI.
///
/// Each slot encodes the visual treatment (color, weight, decoration)
/// for a specific UI role. Under `NO_COLOR`, all slots collapse to
/// monochrome (default foreground) with only modifier distinctions.
#[derive(Debug, Clone)]
pub struct Theme {
    // ── Base text ─────────────────────────────────────────────────
    pub fg_default: Style,
    pub fg_muted: Style,
    pub fg_emphasis: Style,

    // ── Status indicators ─────────────────────────────────────────
    pub status_success: Style,
    pub status_warning: Style,
    pub status_error: Style,
    pub status_running: Style,
    pub status_info: Style,

    // ── Surfaces & borders ────────────────────────────────────────
    pub surface_modal: Style,
    pub border: Style,
    pub border_danger: Style,
    pub border_accent: Style,
    pub input_active: Style,
    pub input_muted: Style,

    // ── Tool cards ────────────────────────────────────────────────
    pub tool_read: Style,
    pub tool_edit: Style,
    pub tool_write: Style,
    pub tool_bash: Style,
    pub tool_search_grep: Style,
    pub tool_github: Style,
    pub tool_task: Style,
    pub tool_git_read: Style,
    pub tool_git_write: Style,

    // ── Task state badges ─────────────────────────────────────────
    pub task_todo: Style,
    pub task_in_progress: Style,
    pub task_blocked: Style,
    pub task_in_review: Style,
    pub task_done: Style,
    pub task_cancelled: Style,

    // ── Conversation roles ────────────────────────────────────────
    pub role_user: Style,
    pub role_assistant: Style,
}

/// Whether the terminal should use color or monochrome output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeMode {
    /// Full color theme (default).
    Color,
    /// Monochrome / no-color mode — driven by the `NO_COLOR` env var.
    NoColor,
}

/// Detect the theme mode from the environment.
///
/// Returns [`ThemeMode::NoColor`] when the `NO_COLOR` environment
/// variable is set (to any value, including empty). Otherwise returns
/// [`ThemeMode::Color`].
pub fn theme_mode_from_env() -> ThemeMode {
    if std::env::var("NO_COLOR").is_ok() {
        ThemeMode::NoColor
    } else {
        ThemeMode::Color
    }
}

/// Build and return the current theme based on the environment.
///
/// Checks `NO_COLOR` at every call. In no-color mode, all foreground
/// slots use `Color::Reset` and the only distinctions come from
/// `Modifier::BOLD`, `Modifier::DIM`, and `Modifier::REVERSED`.
pub fn current_theme() -> Theme {
    let mode = theme_mode_from_env();
    current_theme_for_mode(mode)
}

/// Build a theme for a specific mode (testable without env vars).
pub fn current_theme_for_mode(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::NoColor => no_color_theme(),
        ThemeMode::Color => color_theme(),
    }
}

/// Full-color theme — the default.
fn color_theme() -> Theme {
    Theme {
        // Base text
        fg_default: Style::default().fg(Color::White),
        fg_muted: Style::default().fg(Color::DarkGray),
        fg_emphasis: Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),

        // Status
        status_success: Style::default().fg(Color::Green),
        status_warning: Style::default().fg(Color::Yellow),
        status_error: Style::default().fg(Color::Red),
        status_running: Style::default().fg(Color::Yellow),
        status_info: Style::default().fg(Color::Cyan),

        // Surfaces & borders
        surface_modal: Style::default().bg(Color::Black),
        border: Style::default(),
        border_danger: Style::default().fg(Color::Red),
        border_accent: Style::default().fg(Color::Cyan),
        input_active: Style::default().fg(Color::White),
        input_muted: Style::default().fg(Color::DarkGray),

        // Tool cards
        tool_read: Style::default().fg(Color::Blue),
        tool_edit: Style::default().fg(Color::Magenta),
        tool_write: Style::default().fg(Color::Green),
        tool_bash: Style::default().fg(Color::Yellow),
        tool_search_grep: Style::default().fg(Color::Cyan),
        tool_github: Style::default().fg(Color::Rgb(255, 255, 255)),
        tool_task: Style::default().fg(Color::Rgb(255, 200, 0)),
        tool_git_read: Style::default().fg(Color::Rgb(100, 180, 100)),
        tool_git_write: Style::default().fg(Color::Rgb(255, 140, 0)),

        // Task state badges
        task_todo: Style::default().fg(Color::Gray),
        task_in_progress: Style::default().fg(Color::Yellow),
        task_blocked: Style::default().fg(Color::Red),
        task_in_review: Style::default().fg(Color::Blue),
        task_done: Style::default().fg(Color::Green),
        task_cancelled: Style::default().fg(Color::DarkGray),

        // Conversation roles
        role_user: Style::default().fg(Color::Green),
        role_assistant: Style::default().fg(Color::Cyan),
    }
}

/// Monochrome theme — used when `NO_COLOR` is set.
///
/// All colors are stripped. Only **Bold**, **Dim**, and **Reversed**
/// modifiers are used to convey hierarchy and state.
fn no_color_theme() -> Theme {
    let plain = Style::default();
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let reversed = Style::default().add_modifier(Modifier::REVERSED);

    Theme {
        // Base text
        fg_default: plain,
        fg_muted: dim,
        fg_emphasis: bold,

        // Status — use bold/dim/reversed to differentiate
        status_success: bold,
        status_warning: bold,
        status_error: reversed,
        status_running: bold,
        status_info: dim,

        // Surfaces & borders
        surface_modal: Style::default().bg(Color::Reset),
        border: dim,
        border_danger: bold,
        border_accent: bold,
        input_active: plain,
        input_muted: dim,

        // Tool cards — use bold to distinguish from body text
        tool_read: bold,
        tool_edit: bold,
        tool_write: bold,
        tool_bash: bold,
        tool_search_grep: bold,
        tool_github: bold,
        tool_task: bold,
        tool_git_read: bold,
        tool_git_write: bold,

        // Task state badges — use bold/dim/reversed
        task_todo: dim,
        task_in_progress: bold,
        task_blocked: reversed,
        task_in_review: bold,
        task_done: bold,
        task_cancelled: dim,

        // Conversation roles — use bold/dim
        role_user: bold,
        role_assistant: plain,
    }
}

/// Return the tool card style for a given tool name using the provided theme.
pub fn tool_card_style(name: &str, theme: &Theme) -> Style {
    match name {
        "read" => theme.tool_read,
        "edit" => theme.tool_edit,
        "write" => theme.tool_write,
        "bash" => theme.tool_bash,
        "search_grep" => theme.tool_search_grep,
        "git_read" => theme.tool_git_read,
        "git_write" => theme.tool_git_write,
        "github" => theme.tool_github,
        "task" => theme.tool_task,
        _ => theme.fg_muted,
    }
}

/// Return the task state style for a given state name using the provided theme.
pub fn task_state_style(state: &str, theme: &Theme) -> Style {
    match state {
        "todo" => theme.task_todo,
        "in_progress" => theme.task_in_progress,
        "blocked" => theme.task_blocked,
        "in_review" => theme.task_in_review,
        "done" => theme.task_done,
        "cancelled" | "canceled" => theme.task_cancelled,
        _ => theme.task_cancelled,
    }
}

/// Convert a raw task state string (e.g. `"in_progress"`) into a
/// human-readable display label (e.g. `"In Progress"`).
///
/// Falls back to capitalising the first character of the input.
pub fn format_task_state(state: &str) -> String {
    match state {
        "backlog" => "Backlog".into(),
        "todo" => "To Do".into(),
        "in_progress" => "In Progress".into(),
        "in_review" => "In Review".into(),
        "staged" | "ready" => "Ready".into(),
        "done" => "Done".into(),
        "cancelled" | "canceled" => "Cancelled".into(),
        "blocked" => "Blocked".into(),
        _ => {
            if state.is_empty() {
                "Unknown".into()
            } else {
                let mut chars = state.chars();
                let mut s = String::with_capacity(state.len());
                match chars.next() {
                    Some(c) => {
                        s.push(c.to_ascii_uppercase());
                        s.push_str(chars.as_str());
                    }
                    None => s.push_str("Unknown"),
                }
                s
            }
        }
    }
}

/// Return a semantic style for a task priority level.
pub fn priority_style(priority: &hackpi_tasks::TaskPriority, theme: &Theme) -> Style {
    use hackpi_tasks::TaskPriority;
    match priority {
        TaskPriority::None => theme.fg_muted,
        TaskPriority::Low => theme.fg_default,
        TaskPriority::Medium => theme.fg_emphasis,
        TaskPriority::High => theme.status_warning,
        TaskPriority::Urgent => theme.status_error,
    }
}

/// Return a human-readable label for a task priority level.
pub fn priority_label(priority: &hackpi_tasks::TaskPriority) -> &'static str {
    use hackpi_tasks::TaskPriority;
    match priority {
        TaskPriority::None => "None",
        TaskPriority::Low => "Low",
        TaskPriority::Medium => "Medium",
        TaskPriority::High => "High",
        TaskPriority::Urgent => "Urgent",
    }
}

/// Human-readable label for a tool call status.
pub fn tool_status_label(status: &crate::app::ToolCallStatus) -> &'static str {
    match status {
        crate::app::ToolCallStatus::Running => "Running",
        crate::app::ToolCallStatus::Done(result) => match result {
            hackpi_core::tools::ToolResult::Success { .. } => "Success",
            hackpi_core::tools::ToolResult::SystemError { .. } => "Failed",
            hackpi_core::tools::ToolResult::CommandError { .. } => "Failed",
            hackpi_core::tools::ToolResult::Timeout => "Timeout",
            hackpi_core::tools::ToolResult::Cancelled => "Cancelled",
        },
    }
}

/// Status symbol (glyph) for a tool call status.
pub fn tool_status_symbol(status: &crate::app::ToolCallStatus) -> &'static str {
    match status {
        crate::app::ToolCallStatus::Running => "⋯",
        crate::app::ToolCallStatus::Done(result) => match result {
            hackpi_core::tools::ToolResult::Success { .. } => "✓",
            hackpi_core::tools::ToolResult::SystemError { .. } => "✗",
            hackpi_core::tools::ToolResult::CommandError { .. } => "✗",
            hackpi_core::tools::ToolResult::Timeout => "⚠",
            hackpi_core::tools::ToolResult::Cancelled => "⊘",
        },
    }
}

/// Get the status style for a tool call given the theme.
pub fn tool_status_style(status: &crate::app::ToolCallStatus, theme: &Theme) -> Style {
    match status {
        crate::app::ToolCallStatus::Running => theme.status_running,
        crate::app::ToolCallStatus::Done(result) => match result {
            hackpi_core::tools::ToolResult::Success { .. } => theme.status_success,
            hackpi_core::tools::ToolResult::SystemError { .. } => theme.status_error,
            hackpi_core::tools::ToolResult::CommandError { .. } => theme.status_error,
            hackpi_core::tools::ToolResult::Timeout => theme.status_warning,
            hackpi_core::tools::ToolResult::Cancelled => theme.fg_muted,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mutex to serialize tests that mutate environment variables.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_theme_mode_from_env_no_color_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("NO_COLOR", "1");
        assert_eq!(theme_mode_from_env(), ThemeMode::NoColor);
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn test_theme_mode_from_env_no_color_empty() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("NO_COLOR", "");
        assert_eq!(theme_mode_from_env(), ThemeMode::NoColor);
        std::env::remove_var("NO_COLOR");
    }

    #[test]
    fn test_theme_mode_from_env_no_color_not_set() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("NO_COLOR");
        assert_eq!(theme_mode_from_env(), ThemeMode::Color);
    }

    #[test]
    fn test_current_theme_for_mode_color() {
        let theme = current_theme_for_mode(ThemeMode::Color);
        assert_eq!(theme.tool_read.fg, Some(Color::Blue));
        assert_eq!(theme.status_error.fg, Some(Color::Red));
    }

    #[test]
    fn test_current_theme_for_mode_no_color() {
        let theme = current_theme_for_mode(ThemeMode::NoColor);
        // All fg should be reset (no color)
        assert_eq!(theme.tool_read.fg, None);
        assert_eq!(theme.status_error.fg, None);
        // Error status should have reversed modifier
        assert!(
            theme.status_error.add_modifier.contains(Modifier::REVERSED),
            "no-color error should use REVERSED modifier"
        );
    }

    #[test]
    fn test_no_color_theme_has_no_colors() {
        let theme = no_color_theme();
        assert_eq!(theme.fg_default.fg, None);
        assert_eq!(theme.tool_read.fg, None);
        assert_eq!(theme.tool_bash.fg, None);
        assert_eq!(theme.task_blocked.fg, None);
        assert_eq!(theme.role_user.fg, None);
        assert_eq!(theme.status_running.fg, None);
    }

    #[test]
    fn test_color_theme_has_colors() {
        let theme = color_theme();
        assert!(theme.tool_read.fg.is_some());
        assert!(theme.tool_bash.fg.is_some());
        assert!(theme.task_blocked.fg.is_some());
        assert!(theme.role_user.fg.is_some());
        assert!(theme.status_error.fg.is_some());
    }

    #[test]
    fn test_tool_card_style_read() {
        let theme = color_theme();
        assert_eq!(tool_card_style("read", &theme).fg, Some(Color::Blue));
    }

    #[test]
    fn test_tool_card_style_edit() {
        let theme = color_theme();
        assert_eq!(tool_card_style("edit", &theme).fg, Some(Color::Magenta));
    }

    #[test]
    fn test_tool_card_style_bash() {
        let theme = color_theme();
        assert_eq!(tool_card_style("bash", &theme).fg, Some(Color::Yellow));
    }

    #[test]
    fn test_tool_card_style_search_grep() {
        let theme = color_theme();
        assert_eq!(tool_card_style("search_grep", &theme).fg, Some(Color::Cyan));
    }

    #[test]
    fn test_tool_card_style_write() {
        let theme = color_theme();
        assert_eq!(tool_card_style("write", &theme).fg, Some(Color::Green));
    }

    #[test]
    fn test_tool_card_style_unknown() {
        let theme = color_theme();
        assert_eq!(tool_card_style("unknown", &theme).fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_tool_card_style_git_read() {
        let theme = color_theme();
        assert_eq!(
            tool_card_style("git_read", &theme).fg,
            Some(Color::Rgb(100, 180, 100))
        );
    }

    #[test]
    fn test_tool_card_style_git_write() {
        let theme = color_theme();
        assert_eq!(
            tool_card_style("git_write", &theme).fg,
            Some(Color::Rgb(255, 140, 0))
        );
    }

    #[test]
    fn test_tool_card_style_github() {
        let theme = color_theme();
        assert_eq!(
            tool_card_style("github", &theme).fg,
            Some(Color::Rgb(255, 255, 255))
        );
    }

    #[test]
    fn test_tool_card_style_task() {
        let theme = color_theme();
        assert_eq!(
            tool_card_style("task", &theme).fg,
            Some(Color::Rgb(255, 200, 0))
        );
    }

    #[test]
    fn test_task_state_style_todo() {
        let theme = color_theme();
        assert_eq!(task_state_style("todo", &theme).fg, Some(Color::Gray));
    }

    #[test]
    fn test_task_state_style_in_progress() {
        let theme = color_theme();
        assert_eq!(
            task_state_style("in_progress", &theme).fg,
            Some(Color::Yellow)
        );
    }

    #[test]
    fn test_task_state_style_blocked() {
        let theme = color_theme();
        assert_eq!(task_state_style("blocked", &theme).fg, Some(Color::Red));
    }

    #[test]
    fn test_task_state_style_in_review() {
        let theme = color_theme();
        assert_eq!(task_state_style("in_review", &theme).fg, Some(Color::Blue));
    }

    #[test]
    fn test_task_state_style_done() {
        let theme = color_theme();
        assert_eq!(task_state_style("done", &theme).fg, Some(Color::Green));
    }

    #[test]
    fn test_task_state_style_cancelled() {
        let theme = color_theme();
        assert_eq!(
            task_state_style("cancelled", &theme).fg,
            Some(Color::DarkGray)
        );
    }

    #[test]
    fn test_task_state_style_unknown() {
        let theme = color_theme();
        assert_eq!(
            task_state_style("unknown_state", &theme).fg,
            Some(Color::DarkGray)
        );
    }

    #[test]
    fn test_tool_status_label_running() {
        let status = crate::app::ToolCallStatus::Running;
        assert_eq!(tool_status_label(&status), "Running");
    }

    #[test]
    fn test_tool_status_label_success() {
        let status = crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::Success {
            content: "ok".into(),
        });
        assert_eq!(tool_status_label(&status), "Success");
    }

    #[test]
    fn test_tool_status_label_error() {
        let status =
            crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::SystemError {
                message: "err".into(),
            });
        assert_eq!(tool_status_label(&status), "Failed");
    }

    #[test]
    fn test_tool_status_label_timeout() {
        let status = crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::Timeout);
        assert_eq!(tool_status_label(&status), "Timeout");
    }

    #[test]
    fn test_tool_status_label_cancelled() {
        let status = crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::Cancelled);
        assert_eq!(tool_status_label(&status), "Cancelled");
    }

    #[test]
    fn test_tool_status_label_command_error() {
        let status =
            crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::CommandError {
                content: "cmd failed".into(),
                exit_code: 1,
            });
        assert_eq!(tool_status_label(&status), "Failed");
    }

    #[test]
    fn test_tool_status_symbol_command_error() {
        let status =
            crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::CommandError {
                content: "cmd failed".into(),
                exit_code: 1,
            });
        assert_eq!(tool_status_symbol(&status), "✗");
    }

    #[test]
    fn test_tool_status_style_command_error() {
        let theme = color_theme();
        let status =
            crate::app::ToolCallStatus::Done(hackpi_core::tools::ToolResult::CommandError {
                content: "cmd failed".into(),
                exit_code: 127,
            });
        let style = tool_status_style(&status, &theme);
        assert_eq!(style, theme.status_error);
    }

    // ── format_task_state tests ──────────────────────────────────────────

    #[test]
    fn test_format_task_state_backlog() {
        assert_eq!(format_task_state("backlog"), "Backlog");
    }

    #[test]
    fn test_format_task_state_todo() {
        assert_eq!(format_task_state("todo"), "To Do");
    }

    #[test]
    fn test_format_task_state_in_progress() {
        assert_eq!(format_task_state("in_progress"), "In Progress");
    }

    #[test]
    fn test_format_task_state_in_review() {
        assert_eq!(format_task_state("in_review"), "In Review");
    }

    #[test]
    fn test_format_task_state_staged() {
        assert_eq!(format_task_state("staged"), "Ready");
    }

    #[test]
    fn test_format_task_state_ready() {
        assert_eq!(format_task_state("ready"), "Ready");
    }

    #[test]
    fn test_format_task_state_done() {
        assert_eq!(format_task_state("done"), "Done");
    }

    #[test]
    fn test_format_task_state_cancelled() {
        assert_eq!(format_task_state("cancelled"), "Cancelled");
    }

    #[test]
    fn test_format_task_state_canceled() {
        assert_eq!(format_task_state("canceled"), "Cancelled");
    }

    #[test]
    fn test_format_task_state_blocked() {
        assert_eq!(format_task_state("blocked"), "Blocked");
    }

    #[test]
    fn test_format_task_state_unknown() {
        let label = format_task_state("custom_state");
        assert_eq!(label, "Custom_state");
    }

    #[test]
    fn test_format_task_state_empty() {
        assert_eq!(format_task_state(""), "Unknown");
    }

    // ── priority_label tests ─────────────────────────────────────────────

    #[test]
    fn test_priority_label_none() {
        assert_eq!(priority_label(&hackpi_tasks::TaskPriority::None), "None");
    }

    #[test]
    fn test_priority_label_low() {
        assert_eq!(priority_label(&hackpi_tasks::TaskPriority::Low), "Low");
    }

    #[test]
    fn test_priority_label_medium() {
        assert_eq!(
            priority_label(&hackpi_tasks::TaskPriority::Medium),
            "Medium"
        );
    }

    #[test]
    fn test_priority_label_high() {
        assert_eq!(priority_label(&hackpi_tasks::TaskPriority::High), "High");
    }

    #[test]
    fn test_priority_label_urgent() {
        assert_eq!(
            priority_label(&hackpi_tasks::TaskPriority::Urgent),
            "Urgent"
        );
    }

    // ── priority_style tests ─────────────────────────────────────────────

    #[test]
    fn test_priority_style_none() {
        let theme = color_theme();
        let style = priority_style(&hackpi_tasks::TaskPriority::None, &theme);
        assert_eq!(style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_priority_style_high() {
        let theme = color_theme();
        let style = priority_style(&hackpi_tasks::TaskPriority::High, &theme);
        assert_eq!(style.fg, Some(Color::Yellow));
    }

    #[test]
    fn test_priority_style_urgent() {
        let theme = color_theme();
        let style = priority_style(&hackpi_tasks::TaskPriority::Urgent, &theme);
        assert_eq!(style.fg, Some(Color::Red));
    }
}
