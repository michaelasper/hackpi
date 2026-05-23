use crate::app::{App, AppView, Severity, UiStatus};
use crate::interaction::app_key_context;
use crate::theme::Theme;
use crate::ui::truncate_to_width;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Spinner frames for the animated loading indicator.
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Minimum terminal width for showing API health.
const MIN_WIDTH_FOR_HEALTH: u16 = 40;

/// Separator between regions.
const REGION_SEP: &str = "  ";

/// Build the activity/state label for the center region.
pub(crate) fn activity_label(status: &UiStatus, loading_frame: usize, max_width: usize) -> String {
    let label = match status {
        UiStatus::Idle => String::new(),
        UiStatus::Generating => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Generating… {frame}")
        }
        UiStatus::RunningTool { name } => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            let max_name = max_width.saturating_sub("Running … ".len() + 3);
            let display_name = truncate_to_width(name, max_name);
            format!("Running {display_name}… {frame}")
        }
        UiStatus::LoadingTasks => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Loading tasks… {frame}")
        }
        UiStatus::WaitingForPermission => "Waiting for permission…".into(),
        UiStatus::Error { message, severity } => {
            let tag = match severity {
                Severity::Info => "INFO",
                Severity::Warning => "WARN",
                Severity::Error => "ERR",
            };
            let prefix_len = format!("[{tag}] ").len();
            let max_msg = max_width.saturating_sub(prefix_len);
            let display = truncate_to_width(message, max_msg);
            format!("[{tag}] {display}")
        }
    };
    truncate_to_width(&label, max_width)
}

/// Build contextual shortcut hints, unbounded.
#[cfg(test)]
pub(crate) fn footer_shortcuts(app: &App) -> String {
    build_shortcuts_impl(app, usize::MAX)
}

/// Build shortcut hints fitting within `max_width` display columns.
fn build_shortcuts_impl(app: &App, max_width: usize) -> String {
    let context = app_key_context(app);
    let bindings = crate::interaction::footer_bindings(context);

    let mut result = String::new();
    let mut current_width = 0;
    let sep_width = 2;
    let mut count = 0;

    for b in bindings.iter() {
        let entry = format!("[{}] {}", b.key, b.action);
        let entry_width = UnicodeWidthStr::width(entry.as_str());

        let needed = if result.is_empty() {
            entry_width
        } else {
            sep_width + entry_width
        };

        if current_width + needed > max_width {
            break;
        }

        if !result.is_empty() {
            result.push_str(REGION_SEP);
            current_width += sep_width;
        }
        result.push_str(&entry);
        current_width += entry_width;
        count += 1;

        if count >= 8 {
            break;
        }
    }
    result
}

/// Return view-specific shortcuts for special views that override the
/// normal key-context-derived bindings.
fn view_shortcuts(app: &App) -> Option<String> {
    match &app.active_view {
        AppView::TaskBoard => Some(format!(
            "[Up/Down] Navigate tasks{REGION_SEP}[Enter] View detail{REGION_SEP}[n] Create task"
        )),
        AppView::TaskGraph => Some(format!("[Esc] Back{REGION_SEP}[Ctrl+C] Interrupt")),
        AppView::Diagnostics => Some(format!("[Esc] Back{REGION_SEP}[Ctrl+L] Clear")),
        _ => None,
    }
}
/// Render the status bar with three fixed priority regions.
///
/// Layout: `[shortcuts]  [activity] ··· [health]`
///
/// Shortcuts and activity share the left/center, health is right-aligned.
/// When there's no activity, shortcuts take the full non-health width.
/// When there is activity, it's shown after shortcuts.
/// Long content (errors, tool names) is truncated by display width.
pub(crate) fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let width = area.width as usize;

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

    let show_health = width >= MIN_WIDTH_FOR_HEALTH as usize;
    let health_label = app.connection_health.label();
    let health_width = UnicodeWidthStr::width(health_label);

    // Special case: task detail shows task ID + health
    if matches!(app.active_view, AppView::TaskDetail(_)) {
        let mut text = String::new();
        if let Some(task) = &app.task_detail_cache {
            text = format!(" Task: {}", task.id);
        }
        if show_health {
            if !text.is_empty() {
                text.push_str(REGION_SEP);
            }
            text.push_str(health_label);
        }
        frame.render_widget(Paragraph::new(Line::raw(text)).style(style), area);
        return;
    }

    // Build activity — limit to non-health width so it doesn't overflow health
    let health_budget = if show_health {
        health_width + REGION_SEP.len()
    } else {
        0
    };
    let content_width = width.saturating_sub(health_budget);
    let activity = activity_label(&app.ui_status, app.loading_frame, content_width);
    let activity_width = UnicodeWidthStr::width(activity.as_str());

    // Info message prefix
    let info_prefix = match &app.info_message {
        Some(msg) => format!("{msg} | "),
        None => String::new(),
    };

    // Budget: shortcuts fill remaining space after activity
    let activity_budget = if activity_width > 0 {
        activity_width + REGION_SEP.len()
    } else {
        0
    };
    let shortcut_budget = content_width.saturating_sub(activity_budget);

    let shortcuts = match view_shortcuts(app) {
        Some(vs) => truncate_to_width(&vs, shortcut_budget),
        None => build_shortcuts_impl(app, shortcut_budget),
    };
    let combined = truncate_to_width(&format!("{info_prefix}{shortcuts}"), shortcut_budget);

    // Build the final Line with Spans (no wrapping)
    let mut spans: Vec<Span> = Vec::new();

    let combined_w = UnicodeWidthStr::width(combined.as_str());

    // Shortcuts (left)
    if !combined.is_empty() {
        spans.push(Span::raw(combined));
    }

    // Activity (center)
    if !activity.is_empty() {
        if !spans.is_empty() {
            spans.push(Span::raw(REGION_SEP.to_string()));
        }
        spans.push(Span::raw(activity));
    }

    // Health (right-aligned with padding)
    if show_health {
        let used = combined_w
            + activity_width
            + if combined_w > 0 && activity_width > 0 {
                REGION_SEP.len()
            } else {
                0
            };
        let padding = width.saturating_sub(used + REGION_SEP.len() + health_width);
        if padding > 0 {
            spans.push(Span::raw(" ".repeat(padding)));
        } else if !spans.is_empty() {
            spans.push(Span::raw(REGION_SEP.to_string()));
        }
        spans.push(Span::raw(health_label.to_string()));
    }

    let line = if spans.is_empty() {
        Line::raw(String::new())
    } else {
        Line::from(spans)
    };

    frame.render_widget(Paragraph::new(line).style(style), area);
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, ConnectionHealth, Severity, UiStatus};
    use crate::theme::{current_theme_for_mode, ThemeMode};
    use ratatui::backend::TestBackend;

    fn render_status_row(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let theme = current_theme_for_mode(ThemeMode::NoColor);

        terminal
            .draw(|f| {
                let area = f.area();
                let status_area =
                    ratatui::layout::Rect::new(area.x, area.y + area.height - 1, area.width, 1);
                render_status(f, status_area, app, &theme);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row_start = (height as usize - 1) * (width as usize);
        buffer.content[row_start..row_start + width as usize]
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn test_activity_label_idle_is_empty() {
        assert!(activity_label(&UiStatus::Idle, 0, 80).is_empty());
    }
    #[test]
    fn test_activity_label_generating() {
        assert!(activity_label(&UiStatus::Generating, 0, 80).contains("Generating"));
    }
    #[test]
    fn test_activity_label_running_tool() {
        assert!(activity_label(
            &UiStatus::RunningTool {
                name: "bash".into()
            },
            0,
            80
        )
        .contains("bash"));
    }
    #[test]
    fn test_activity_label_error_tag() {
        assert!(activity_label(
            &UiStatus::Error {
                message: "x".into(),
                severity: Severity::Error
            },
            0,
            80
        )
        .contains("[ERR]"));
    }
    #[test]
    fn test_activity_label_permission() {
        assert!(activity_label(&UiStatus::WaitingForPermission, 0, 80).contains("permission"));
    }
    #[test]
    fn test_activity_label_loading() {
        assert!(activity_label(&UiStatus::LoadingTasks, 0, 80).contains("Loading tasks"));
    }
    #[test]
    fn test_footer_shortcuts_max_eight() {
        assert!(footer_shortcuts(&App::new()).matches(']').count() <= 8);
    }
    #[test]
    fn test_build_shortcuts_width_bound() {
        let s = build_shortcuts_impl(&App::new(), 40);
        assert!(UnicodeWidthStr::width(s.as_str()) <= 40);
    }

    // Standard sizes
    #[test]
    fn test_render_80x24_idle() {
        let row = render_status_row(&App::new(), 80, 24);
        assert!(row.contains("Ctrl+C"), "shortcuts: {row}");
        assert!(row.contains("API: unknown"), "health: {row}");
        assert!(!row.contains('\n'));
    }
    #[test]
    fn test_render_80x24_generating() {
        let mut app = App::new();
        app.ui_status = UiStatus::Generating;
        let row = render_status_row(&app, 80, 24);
        assert!(row.contains("Generating"), "activity: {row}");
        assert!(row.contains("API: unknown"), "health: {row}");
    }
    #[test]
    fn test_render_80x24_error() {
        let mut app = App::new();
        app.ui_status = UiStatus::Error {
            message: "API timeout".into(),
            severity: Severity::Error,
        };
        let row = render_status_row(&app, 80, 24);
        assert!(row.contains("[ERR]"), "ERR: {row}");
        assert!(row.contains("API timeout"), "msg: {row}");
        assert!(row.contains("API: unknown"), "health: {row}");
    }
    #[test]
    fn test_render_120x40_idle() {
        let row = render_status_row(&App::new(), 120, 40);
        assert!(row.contains("API: unknown"), "health: {row}");
    }
    #[test]
    fn test_render_200x60_idle() {
        assert!(render_status_row(&App::new(), 200, 60).contains("API: unknown"));
    }

    // Long messages
    #[test]
    fn test_long_error_health_120() {
        let mut app = App::new();
        app.ui_status = UiStatus::Error {
            message: "A".repeat(200),
            severity: Severity::Error,
        };
        let row = render_status_row(&app, 120, 40);
        assert!(row.contains("API: unknown"), "health not found in: {row}");
    }
    #[test]
    fn test_long_error_health_80() {
        let mut app = App::new();
        app.ui_status = UiStatus::Error {
            message: "X".repeat(200),
            severity: Severity::Warning,
        };
        assert!(render_status_row(&app, 80, 24).contains("API: unknown"));
    }
    #[test]
    fn test_long_tool_health_80() {
        let mut app = App::new();
        app.ui_status = UiStatus::RunningTool {
            name: "A".repeat(100),
        };
        assert!(render_status_row(&app, 80, 24).contains("API: unknown"));
    }
    #[test]
    fn test_long_info_health_80() {
        let mut app = App::new();
        app.info_message = Some("Z".repeat(200));
        assert!(render_status_row(&app, 80, 24).contains("API: unknown"));
    }

    // Narrow terminal
    #[test]
    fn test_narrow_30_drops_health() {
        assert!(!render_status_row(&App::new(), 30, 24).contains("API:"));
    }
    #[test]
    fn test_at_60_shows_health() {
        assert!(render_status_row(&App::new(), 60, 24).contains("API: unknown"));
    }
    #[test]
    fn test_at_39_drops_health() {
        assert!(!render_status_row(&App::new(), 39, 24).contains("API:"));
    }

    // Distinct states
    #[test]
    fn test_generating() {
        let mut app = App::new();
        app.ui_status = UiStatus::Generating;
        assert!(render_status_row(&app, 120, 40).contains("Generating"));
    }
    #[test]
    fn test_running_tool() {
        let mut app = App::new();
        app.ui_status = UiStatus::RunningTool {
            name: "bash".into(),
        };
        assert!(render_status_row(&app, 120, 40).contains("bash"));
    }
    #[test]
    fn test_permission() {
        let mut app = App::new();
        app.ui_status = UiStatus::WaitingForPermission;
        assert!(render_status_row(&app, 120, 40).contains("permission"));
    }
    #[test]
    fn test_loading() {
        let mut app = App::new();
        app.ui_status = UiStatus::LoadingTasks;
        assert!(render_status_row(&app, 120, 40).contains("Loading tasks"));
    }
    #[test]
    fn test_task_board() {
        let mut app = App::new();
        app.active_view = AppView::TaskBoard;
        let row = render_status_row(&app, 120, 40);
        assert!(row.contains("Navigate tasks") || row.contains("Detail") || row.contains("Create"));
    }
    #[test]
    fn test_task_detail() {
        let mut app = App::new();
        app.active_view = AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Test".into(),
            description: String::new(),
            state: "todo".into(),
            priority: hackpi_tasks::TaskPriority::None,
            workflow: "default".into(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        let row = render_status_row(&app, 120, 40);
        assert!(row.contains("TSK-001"));
        assert!(row.contains("API: unknown"));
    }
    #[test]
    fn test_connected() {
        let mut app = App::new();
        app.connection_health = ConnectionHealth::Connected;
        assert!(render_status_row(&app, 200, 60).contains("API: connected"));
    }
    #[test]
    fn test_error_health() {
        let mut app = App::new();
        app.connection_health = ConnectionHealth::Error {
            message: "timeout".into(),
        };
        assert!(render_status_row(&app, 200, 60).contains("API: error"));
    }
    #[test]
    fn test_offline() {
        let mut app = App::new();
        app.connection_health = ConnectionHealth::Offline;
        assert!(render_status_row(&app, 200, 60).contains("API: offline"));
    }
    #[test]
    fn test_info_message() {
        let mut app = App::new();
        app.info_message = Some("Created TSK-003".to_string());
        assert!(render_status_row(&app, 120, 40).contains("Created TSK-003"));
    }
    #[test]
    fn test_long_info_truncated() {
        let mut app = App::new();
        app.info_message = Some("A".repeat(200));
        let row = render_status_row(&app, 80, 24);
        assert!(!row.contains('\n'));
        assert!(row.contains("API: unknown"));
    }
}
