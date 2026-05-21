use crate::app::{App, AppView, Severity, UiStatus};
use crate::interaction::{app_key_context, footer_bindings};
use crate::theme::Theme;
use crate::ui::truncate_for_display;
use ratatui::{layout::Rect, text::Line, widgets::Paragraph, Frame};

/// Spinner frames for the animated loading indicator.
/// Cycles through these while waiting for LLM response.
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Build the status text for the UiStatus indicator portion of the status bar.
pub(crate) fn ui_status_label(status: &UiStatus, loading_frame: usize) -> String {
    match status {
        UiStatus::Idle => String::new(),
        UiStatus::Generating => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Generating… {frame}")
        }
        UiStatus::RunningTool { name } => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Running {name}… {frame}")
        }
        UiStatus::LoadingTasks => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Loading tasks... {frame}")
        }
        UiStatus::WaitingForPermission => "Waiting for permission…".into(),
        UiStatus::Error { message, severity } => {
            let tag = match severity {
                Severity::Info => "INFO",
                Severity::Warning => "WARN",
                Severity::Error => "ERR",
            };
            // Truncate long error messages for the status bar (char-safe)
            let display = truncate_for_display(message, 50);
            format!("[{tag}] {display}")
        }
    }
}

pub(crate) fn status_bar_text(app: &App) -> String {
    let context = app_key_context(app);
    let is_detail = matches!(app.active_view, AppView::TaskDetail(_));

    // Task detail shows a bespoke status line with task ID
    if is_detail {
        if let Some(task) = &app.task_detail_cache {
            return format!(" Task: {}  {}", task.id, app.connection_health.label());
        }
    }

    // State indicator
    let state_text = ui_status_label(&app.ui_status, app.loading_frame);

    // Dynamic footer hints from KEY_BINDINGS table
    let bindings = footer_bindings(context);
    let binding_text: String = bindings
        .iter()
        .map(|b| format!("[{}] {}", b.key, b.action))
        .collect::<Vec<_>>()
        .join("  ");

    // Extra info message if present
    let info_prefix = match &app.info_message {
        Some(msg) => format!("{msg} | "),
        None => String::new(),
    };

    let health_label = app.connection_health.label();

    if !state_text.is_empty() {
        if !binding_text.is_empty() {
            format!(" {info_prefix}{state_text}  ·  {binding_text}  {health_label}")
        } else {
            format!(" {info_prefix}{state_text}  {health_label}")
        }
    } else if !binding_text.is_empty() {
        format!(" {info_prefix}{binding_text}  {health_label}")
    } else if !info_prefix.is_empty() {
        format!(" {info_prefix}{health_label}")
    } else {
        format!(" {health_label}")
    }
}

pub(crate) fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = status_bar_text(app);

    // Derive style from UiStatus
    let style = match &app.ui_status {
        UiStatus::Idle | UiStatus::WaitingForPermission => theme.fg_muted,
        UiStatus::Generating | UiStatus::RunningTool { .. } | UiStatus::LoadingTasks => {
            theme.status_running
        }
        UiStatus::Error {
            severity: Severity::Info,
            ..
        } => theme.status_info,
        UiStatus::Error {
            severity: Severity::Warning,
            ..
        } => theme.status_warning,
        UiStatus::Error {
            severity: Severity::Error,
            ..
        } => theme.status_error,
    };

    // Use Line::raw (no wrapping) so the status bar never wraps to the next
    // line, which would overlap with other layout regions on narrow terminals.
    frame.render_widget(Paragraph::new(Line::from(text)).style(style), area);
}
