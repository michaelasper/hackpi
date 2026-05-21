pub mod conversation;
pub mod diagnostics;
pub mod input;
pub mod layout;
pub mod modals;
pub mod status;
pub mod task_board;

// ── Re-exports: public API ────────────────────────────────────────────────
pub use input::{cursor_position_for_input, display_width_prefix, truncate_to_width};
pub use layout::{
    centered_rect, is_too_small, modal_rect, render_too_small, split_root, RootLayout,
    MIN_TERMINAL_HEIGHT, MIN_TERMINAL_WIDTH,
};

// ── Crate-visible re-exports (used by tests and sub-modules) ─────────────
pub(crate) use conversation::render_conversation;
pub(crate) use diagnostics::render_diagnostics;
pub(crate) use modals::{
    render_autocomplete_modal, render_help_overlay, render_permission_modal,
    render_task_create_prompt,
};
pub(crate) use status::render_status;
pub(crate) use task_board::{render_task_board, render_task_detail, render_task_graph};

use crate::app::{App, AppView};
use crate::theme::{current_theme, Theme};
use crate::ui::input::render_input;
use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

// ── Main render entry point ────────────────────────────────────────────────

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Gate: if terminal is too small, show resize message instead of layout
    if is_too_small(area) {
        render_too_small(frame, area);
        return;
    }

    let theme = current_theme();
    let root = split_root(area);

    render_tab_header(frame, root.header, &app.active_view, app, &theme);

    match &app.active_view {
        AppView::Conversation => {
            render_conversation(frame, root.main, app, &theme);
        }
        AppView::TaskDetail(_) => {
            render_task_detail(frame, root.main, app, &theme);
        }
        AppView::TaskBoard => {
            render_task_board(frame, root.main, app, &theme);
        }
        AppView::TaskGraph => {
            render_task_graph(frame, root.main, app, &theme);
        }
        AppView::Diagnostics => {
            render_diagnostics(frame, root.main, app, &theme);
        }
    }

    render_input(frame, root.input, app, &theme);
    render_status(frame, root.status, app, &theme);

    if app.autocomplete_visible {
        render_autocomplete_modal(frame, root.input, app, &theme);
    }

    if app.pending_permission.is_some() {
        render_permission_modal(frame, area, app, &theme);
    }

    if app.creating_task {
        render_task_create_prompt(frame, root.input, app, &theme);
    }

    if app.help_visible {
        render_help_overlay(frame, area, app, &theme);
    }
}

/// Render the tab header with active/inactive tab highlighting and version/usage info.
fn render_tab_header(
    frame: &mut Frame,
    area: Rect,
    active_view: &AppView,
    app: &App,
    theme: &Theme,
) {
    let diag_count = app.diagnostics.len();
    let tabs = [
        ("Conv", matches!(active_view, AppView::Conversation)),
        (
            "Tasks",
            matches!(active_view, AppView::TaskBoard | AppView::TaskDetail(_)),
        ),
        ("Graph", matches!(active_view, AppView::TaskGraph)),
        (
            &format!(
                "Diag{}",
                if diag_count > 0 {
                    format!("({diag_count})")
                } else {
                    String::new()
                }
            ),
            matches!(active_view, AppView::Diagnostics),
        ),
    ];

    let mut spans: Vec<Span> = Vec::new();
    for (i, (label, is_active)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[Tab] {label}"),
            if *is_active {
                theme.fg_emphasis.add_modifier(Modifier::UNDERLINED)
            } else {
                theme.fg_muted
            },
        ));
    }

    // Right-align version and usage info
    let usage_text = match &app.usage {
        Some(u) => format!("{}↑ {}↓", u.input_tokens, u.output_tokens),
        None => "0↑ 0↓".into(),
    };
    let version = env!("CARGO_PKG_VERSION");
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("hackpi v{version} · {usage_text}"),
        theme.fg_muted,
    ));

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Count the total number of visual rows a `Text` will occupy when rendered
/// in an area of the given width, accounting for word wrapping.
pub(crate) fn count_visual_lines(text: &Text, area_width: usize) -> usize {
    if area_width == 0 {
        return text.lines.len();
    }
    text.lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1 // empty line occupies one row
            } else {
                line_width.div_ceil(area_width)
            }
        })
        .sum()
}

/// Truncate a string to at most `max_len` characters, appending "…" if truncated.
pub(crate) fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ConnectionHealth, Severity, UiStatus};
    use crate::events::TuiEvent;
    use crate::ui::conversation::{assistant_prefix, user_prefix};
    use crate::ui::status::status_bar_text;

    #[test]
    fn test_tab_header_shows_version_and_usage() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.usage = Some(hackpi_core::types::Usage {
            input_tokens: 150,
            output_tokens: 75,
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("150↑"),
            "tab header should show usage, got: {cell_str}"
        );
        assert!(
            cell_str.contains("75↓"),
            "tab header should show usage, got: {cell_str}"
        );
    }

    #[test]
    fn test_tab_header_shows_zero_usage() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("0↑"),
            "tab header should show 0↑, got: {cell_str}"
        );
        assert!(
            cell_str.contains("0↓"),
            "tab header should show 0↓, got: {cell_str}"
        );
    }

    #[test]
    fn test_conversation_user_prefix_includes_me_label() {
        let prefix = user_prefix();
        assert_eq!(prefix, " ○ me: ");
    }

    #[test]
    fn test_conversation_assistant_prefix_includes_assistant_label() {
        let prefix = assistant_prefix();
        assert_eq!(prefix, " ● assistant: ");
    }

    #[test]
    fn test_status_bar_resting_shows_bindings() {
        let app = App::new();
        let text = status_bar_text(&app);
        assert!(
            text.contains("Interrupt generation"),
            "status should show Interrupt generation: {text}"
        );
        assert!(
            text.contains("Clear conversation"),
            "status should show Clear conversation: {text}"
        );
        assert!(text.contains("Exit"), "status should show Exit: {text}");
    }

    #[test]
    fn test_status_bar_generating_shows_interrupt_hint() {
        let mut app = App::new();
        app.ui_status = UiStatus::Generating;
        let text = status_bar_text(&app);
        assert!(
            text.contains("Generating"),
            "status should show Generating: {text}"
        );
        assert!(
            text.contains("Interrupt generation"),
            "should show interrupt hint via context bindings: {text}"
        );
    }

    #[test]
    fn test_status_bar_error_shows_ui_status() {
        let mut app = App::new();
        app.ui_status = UiStatus::Error {
            message: "API timeout".into(),
            severity: Severity::Error,
        };
        let text = status_bar_text(&app);
        assert!(text.contains("ERR"), "status should show ERR tag: {text}");
        assert!(
            text.contains("API timeout"),
            "status should show error message: {text}"
        );
    }

    #[test]
    fn test_status_bar_running_tool_shows_name() {
        let mut app = App::new();
        app.ui_status = UiStatus::RunningTool {
            name: "bash".into(),
        };
        let text = status_bar_text(&app);
        assert!(
            text.contains("Running bash"),
            "status should show running tool name: {text}"
        );
    }

    #[test]
    fn test_status_bar_loading_tasks_shows_spinner() {
        let mut app = App::new();
        app.ui_status = UiStatus::LoadingTasks;
        let text = status_bar_text(&app);
        assert!(
            text.contains("Loading tasks"),
            "status should show loading: {text}"
        );
    }

    #[test]
    fn test_status_bar_includes_connection_indicator() {
        let app = App::new();
        let text = status_bar_text(&app);
        assert!(
            text.contains("unknown"),
            "status bar should show 'unknown' health by default, got: {text}"
        );
    }

    #[test]
    fn test_connection_health_label_unknown() {
        assert_eq!(ConnectionHealth::Unknown.label(), "API: unknown");
    }

    #[test]
    fn test_connection_health_label_connected() {
        assert_eq!(ConnectionHealth::Connected.label(), "API: connected");
    }

    #[test]
    fn test_connection_health_label_error() {
        assert_eq!(
            ConnectionHealth::Error {
                message: "err".into()
            }
            .label(),
            "API: error"
        );
    }

    #[test]
    fn test_connection_health_label_offline() {
        assert_eq!(ConnectionHealth::Offline.label(), "API: offline");
    }

    // ── Tool card style tests (delegated to theme module) ──────────────
    // See tests in theme.rs for tool_card_style(), task_state_style(), etc.

    #[test]
    fn test_render_permission_modal_does_not_panic_with_pending_prompt() {
        // Create a minimal test buffer to render into
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: false,
        });

        // render() should not panic even with a pending permission prompt
        terminal.draw(|f| render(f, &app)).unwrap();

        // Verify the buffer contains expected modal text
        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Permission Required"),
            "modal should show title, got: {cell_str:?}"
        );
        assert!(
            cell_str.contains("Allow once"),
            "modal should show Allow once option"
        );
        assert!(
            cell_str.contains("Allow until exit"),
            "modal should show Allow until exit option"
        );
        assert!(cell_str.contains("Deny"), "modal should show Deny option");
        assert!(
            cell_str.contains("Always allow"),
            "modal should show Always allow option"
        );
        assert!(
            cell_str.contains("Always deny"),
            "modal should show Always deny option"
        );
        assert!(
            cell_str.contains("Esc"),
            "modal should show Esc key binding"
        );
    }

    #[test]
    fn test_render_permission_modal_shows_this_request_group() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "ls -la".into(),
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: false,
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // "This request" group contains Allow once and Deny
        assert!(
            cell_str.contains("This request"),
            "modal should show 'This request' heading"
        );
        assert!(
            cell_str.contains("[1] Allow once"),
            "modal should show [1] Allow once"
        );
        assert!(cell_str.contains("[3] Deny"), "modal should show [3] Deny");
        assert!(
            cell_str.contains("[2] Allow until exit"),
            "modal should show [2] Allow until exit"
        );
        assert!(
            cell_str.contains("[4] Always allow"),
            "modal should show [4] Always allow this pattern"
        );
        assert!(
            cell_str.contains("[5] Always deny"),
            "modal should show [5] Always deny this pattern"
        );
        assert!(
            cell_str.contains("[Esc] Deny"),
            "modal should show [Esc] Deny"
        );
    }

    #[test]
    fn test_render_permission_modal_shows_confirmation_when_confirming() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: true, // User pressed 4 once
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // Confirmation text should appear instead of normal "Always allow"
        assert!(
            cell_str.contains("Press 4 again"),
            "modal should show confirmation prompt when confirming_always_allow is true"
        );
        assert!(
            !cell_str.contains("Always allow this pattern"),
            "modal should NOT show normal 'Always allow' text during confirmation"
        );
        // Other options should still be present
        assert!(
            cell_str.contains("[1] Allow once"),
            "other options should still be visible during confirmation"
        );
        assert!(
            cell_str.contains("[3] Deny"),
            "Deny should still be visible during confirmation"
        );
        assert!(
            cell_str.contains("[5] Always deny"),
            "Always deny should still be visible during confirmation"
        );
    }

    #[test]
    fn test_render_permission_modal_shows_all_groups_at_80x24() {
        use ratatui::backend::TestBackend;
        // 80x24 is the minimum supported terminal size
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::FileProtection,
            tool: "read".into(),
            details: ".env".into(),
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: false,
        });

        // Must not panic at 80x24
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // All groups must be visible
        assert!(
            cell_str.contains("Permission Required"),
            "modal title should be visible at 80x24"
        );
        assert!(
            cell_str.contains("This request"),
            "'This request' group should be visible at 80x24"
        );
        assert!(
            cell_str.contains("This session"),
            "'This session' group should be visible at 80x24"
        );
        assert!(
            cell_str.contains("Persistent rule"),
            "'Persistent rule' group should be visible at 80x24"
        );
        assert!(
            cell_str.contains("Esc"),
            "Esc hint should be visible at 80x24"
        );
    }

    #[test]
    fn test_render_permission_modal_long_values_wrapped_at_80x24() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let long_details = "x".repeat(300);
        let long_tool = "y".repeat(100);
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::PathAccess,
            tool: long_tool,
            details: long_details,
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: false,
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // Long values should NOT be silently truncated — at least 50 contiguous
        // characters should be visible in the buffer (wrapping, not ellipsis).
        assert!(
            cell_str.contains(&"x".repeat(50)),
            "long details should be wrapped and visible, got: {:?}",
            cell_str.chars().take(200).collect::<String>()
        );
        assert!(
            cell_str.contains(&"y".repeat(30)),
            "long tool name should be wrapped and visible, got: {:?}",
            cell_str.chars().take(200).collect::<String>()
        );
    }

    #[test]
    fn test_render_permission_modal_shows_esc_deny() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: "bash".into(),
            details: "rm -rf /".into(),
        };

        let mut app = App::new();
        app.pending_permission = Some(crate::app::PermissionPrompt {
            id: 1,
            reason,
            response: Some(tx),
            confirming_always_allow: false,
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        assert!(
            cell_str.contains("[Esc] Deny"),
            "modal should show 'Esc = Deny', got: {cell_str:?}"
        );
    }

    // ── Task board view tests ──────────────────────────────────────────

    #[test]
    fn test_render_tab_header_conversation_active() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        assert!(matches!(app.active_view, crate::app::AppView::Conversation));

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(cell_str.contains("Conv"), "tab header should show Conv tab");
        assert!(
            cell_str.contains("Tasks"),
            "tab header should show Tasks tab"
        );
        assert!(
            cell_str.contains("Graph"),
            "tab header should show Graph tab"
        );
        assert!(cell_str.contains("Diag"), "tab header should show Diag tab");
    }

    #[test]
    fn test_render_task_board_empty_state() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("No tasks yet"),
            "task board should show updated empty state message, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_board_grouped_by_state() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![
            hackpi_tasks::Task {
                id: "TSK-001".to_string(),
                title: "Implement auth".to_string(),
                description: String::new(),
                state: "in_progress".to_string(),
                priority: hackpi_tasks::TaskPriority::High,
                workflow: "default".to_string(),
                blocked_by: vec![],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            hackpi_tasks::Task {
                id: "TSK-002".to_string(),
                title: "Write tests".to_string(),
                description: String::new(),
                state: "todo".to_string(),
                priority: hackpi_tasks::TaskPriority::Medium,
                workflow: "default".to_string(),
                blocked_by: vec!["TSK-001".to_string()],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        ];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        // Group headers
        assert!(
            cell_str.contains("In Progress (1)"),
            "task board should show group header with count, got: {cell_str}"
        );
        assert!(
            cell_str.contains("To Do (1)"),
            "task board should show group header with count"
        );
        // Task IDs and titles
        assert!(
            cell_str.contains("TSK-001"),
            "task board should show TSK-001"
        );
        assert!(
            cell_str.contains("TSK-002"),
            "task board should show TSK-002"
        );
        assert!(
            cell_str.contains("Implement auth"),
            "task board should show task title"
        );
        // State labels are now human-readable
        assert!(
            cell_str.contains("[In Progress]"),
            "task board should show human-readable state label, got: {cell_str}"
        );
        assert!(
            cell_str.contains("[To Do]"),
            "task board should show human-readable state label"
        );
        // Blocked-by sub-entries
        assert!(
            cell_str.contains("blocked by TSK-001"),
            "task board should show blocked_by sub-entry"
        );
    }

    #[test]
    fn test_render_task_board_grouped_many_states_at_narrow_width() {
        use ratatui::backend::TestBackend;
        // 80x24 is the minimum terminal size
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![
            hackpi_tasks::Task {
                id: "TSK-001".to_string(),
                title: "Task one".to_string(),
                description: String::new(),
                state: "todo".to_string(),
                priority: hackpi_tasks::TaskPriority::Low,
                workflow: "default".to_string(),
                blocked_by: vec![],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            hackpi_tasks::Task {
                id: "TSK-002".to_string(),
                title: "Task two".to_string(),
                description: String::new(),
                state: "in_progress".to_string(),
                priority: hackpi_tasks::TaskPriority::High,
                workflow: "default".to_string(),
                blocked_by: vec![],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            hackpi_tasks::Task {
                id: "TSK-003".to_string(),
                title: "Task three".to_string(),
                description: String::new(),
                state: "done".to_string(),
                priority: hackpi_tasks::TaskPriority::None,
                workflow: "default".to_string(),
                blocked_by: vec![],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            hackpi_tasks::Task {
                id: "TSK-004".to_string(),
                title: "Blocked task".to_string(),
                description: String::new(),
                state: "blocked".to_string(),
                priority: hackpi_tasks::TaskPriority::Urgent,
                workflow: "default".to_string(),
                blocked_by: vec!["TSK-001".to_string()],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        ];

        // Must not panic at 80x24
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        // All group headers should be present
        assert!(
            cell_str.contains("To Do (1)"),
            "should show To Do group with count"
        );
        assert!(
            cell_str.contains("In Progress (1)"),
            "should show In Progress group"
        );
        assert!(cell_str.contains("Done (1)"), "should show Done group");
        assert!(
            cell_str.contains("Blocked (1)"),
            "should show Blocked group"
        );
        // All task IDs should be visible
        assert!(cell_str.contains("TSK-001"));
        assert!(cell_str.contains("TSK-002"));
        assert!(cell_str.contains("TSK-003"));
        assert!(cell_str.contains("TSK-004"));
        // Blocked-by sub-entry
        assert!(
            cell_str.contains("blocked by TSK-001"),
            "should show blocked-by dependency"
        );
    }

    #[test]
    fn test_render_task_graph_shows_dependency_header() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskGraph;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Task Dependencies"),
            "graph view should show dependency header, got: {cell_str}"
        );
        assert!(
            cell_str.contains("No tasks loaded"),
            "graph view with empty cache should show helpful message, got: {cell_str}"
        );
        assert!(
            !cell_str.contains("coming soon"),
            "graph view should NOT show placeholder text"
        );
    }

    #[test]
    fn test_render_task_graph_with_selected_task_shows_deps() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskGraph;
        app.task_list_cache = vec![
            hackpi_tasks::Task {
                id: "TSK-001".to_string(),
                title: "Implement auth".to_string(),
                description: String::new(),
                state: "in_progress".to_string(),
                priority: hackpi_tasks::TaskPriority::High,
                workflow: "default".to_string(),
                blocked_by: vec!["TSK-002".to_string()],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
            hackpi_tasks::Task {
                id: "TSK-002".to_string(),
                title: "Setup database".to_string(),
                description: String::new(),
                state: "done".to_string(),
                priority: hackpi_tasks::TaskPriority::Medium,
                workflow: "default".to_string(),
                blocked_by: vec![],
                labels: vec![],
                assignee: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            },
        ];
        app.selected_task_idx = 0;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("TSK-001"),
            "graph view should show selected task ID"
        );
        assert!(
            cell_str.contains("Blocked by"),
            "graph view should show blocked-by section"
        );
        assert!(
            cell_str.contains("TSK-002"),
            "graph view should show blocker ID"
        );
        assert!(
            cell_str.contains("Setup database"),
            "graph view should show blocker title"
        );
    }

    #[test]
    fn test_render_task_graph_empty_cache_shows_helpful_message() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskGraph;
        app.task_list_cache = vec![];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("No tasks loaded"),
            "graph view with empty cache should show helpful message, got: {cell_str}"
        );
        assert!(
            !cell_str.contains("coming soon"),
            "graph view should NOT show placeholder text"
        );
    }

    #[test]
    fn test_render_task_detail_shows_task_id_in_status() {
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Test".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        let text = status_bar_text(&app);
        assert!(
            text.contains("TSK-001"),
            "status bar should show task ID in detail view: {text}"
        );
        assert!(
            text.contains("API: unknown"),
            "status bar should include connection indicator text: {text}"
        );
    }

    #[test]
    fn test_render_no_panic_with_task_board_and_empty_cache() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![];
        app.selected_task_idx = 0;

        // Should not panic
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    #[test]
    fn test_render_no_panic_with_selected_idx_out_of_bounds() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Test".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        app.selected_task_idx = 5; // out of bounds

        // Should not panic
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    // ── Tool call title positioning tests ────────────────────────────────

    #[test]
    fn test_tool_call_title_appears_after_user_text_not_at_top() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // Add a user message
        app.handle_event(TuiEvent::Submit("hello".into()));
        // Add an assistant tool call with result
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
            input: None,
        });
        app.handle_event(TuiEvent::ToolResult {
            id: "tc1".into(),
            result: hackpi_core::tools::ToolResult::Success {
                content: "file content".into(),
            },
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // Find the position of the user prefix and the tool call title
        let user_pos = cell_str.find("○ me:").expect("should contain user prefix");
        let title_pos = cell_str
            .find("read")
            .expect("should contain tool call name");

        // The tool call title should appear AFTER the user text,
        // not before it (which would happen if insert(0) bug is present)
        assert!(
            title_pos > user_pos,
            "tool call title 'read' should appear after user text '○ me:', \
             but title_pos={title_pos} <= user_pos={user_pos}. \
             This means insert(0) is pushing the title to the top."
        );
    }

    #[test]
    fn test_multiple_tool_calls_keep_correct_ordering() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // First conversation: user + assistant with tool call
        app.handle_event(TuiEvent::Submit("first message".into()));
        app.handle_event(TuiEvent::StreamChunk("first response".into()));
        app.handle_event(TuiEvent::Done);

        // Second conversation: user + assistant with two tool calls
        app.handle_event(TuiEvent::Submit("second message".into()));
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
            input: None,
        });
        app.handle_event(TuiEvent::ToolResult {
            id: "tc1".into(),
            result: hackpi_core::tools::ToolResult::Success {
                content: "read result".into(),
            },
        });
        app.handle_event(TuiEvent::ToolCall {
            id: "tc2".into(),
            name: "edit".into(),
            input: None,
        });
        app.handle_event(TuiEvent::ToolResult {
            id: "tc2".into(),
            result: hackpi_core::tools::ToolResult::Success {
                content: "edit result".into(),
            },
        });

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // First user message and response should appear before second conversation
        let first_user_pos = cell_str.find("first message").expect("first message");
        let first_resp_pos = cell_str.find("first response").expect("first response");
        let second_user_pos = cell_str.find("second message").expect("second message");
        let tc1_pos = cell_str.find("read").expect("tool call 'read'");
        let tc2_pos = cell_str.find("edit").expect("tool call 'edit'");

        assert!(
            first_resp_pos > first_user_pos,
            "first response after first user"
        );
        assert!(
            second_user_pos > first_resp_pos,
            "second message after first response"
        );
        assert!(
            tc1_pos > second_user_pos,
            "first tool call title after second user message"
        );
        assert!(tc2_pos > tc1_pos, "second tool call title after first");
    }

    // ── Task detail view tests ──────────────────────────────────────────

    /// Helper to create a fully populated task for detail view testing.
    fn make_detail_task() -> hackpi_tasks::Task {
        hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Implement auth module".to_string(),
            description: "Implement JWT-based authentication with refresh tokens.".to_string(),
            state: "in_progress".to_string(),
            priority: hackpi_tasks::TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec!["backend".to_string(), "security".to_string()],
            assignee: Some("alice".to_string()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_render_task_detail_shows_task_title() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Implement auth module"),
            "detail view should show task title, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_state() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("In Progress"),
            "detail view should show human-readable state, got: {cell_str}"
        );
        assert!(
            !cell_str.contains("in_progress"),
            "detail view should NOT show raw snake_case state, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_priority() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("High"),
            "detail view should show human-readable priority, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_labels() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("backend"),
            "detail view should show labels, got: {cell_str}"
        );
        assert!(
            cell_str.contains("security"),
            "detail view should show labels, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_description() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("JWT"),
            "detail view should show description, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_assignee() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("alice"),
            "detail view should show assignee, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_empty_fields_show_dash() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = hackpi_tasks::Task {
            id: "TSK-002".to_string(),
            title: "Simple task".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-002".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        // Empty/None fields should show em dash
        assert!(
            cell_str.contains("—"),
            "detail view should show em dash for empty fields, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_blocked_by() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = hackpi_tasks::Task {
            id: "TSK-002".to_string(),
            title: "Blocked task".to_string(),
            description: String::new(),
            state: "blocked".to_string(),
            priority: hackpi_tasks::TaskPriority::High,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-001".to_string()],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-002".to_string());
        app.task_detail_cache = Some(task);
        app.task_detail_blocked_by = vec![hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Blocker".to_string(),
            description: String::new(),
            state: "in_progress".to_string(),
            priority: hackpi_tasks::TaskPriority::Medium,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("TSK-001"),
            "detail view should show blocked by IDs, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_blocking() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);
        app.task_detail_blocking = vec![hackpi_tasks::Task {
            id: "TSK-003".to_string(),
            title: "Dependent task".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::Medium,
            workflow: "default".to_string(),
            blocked_by: vec!["TSK-001".to_string()],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("TSK-003"),
            "detail view should show blocking IDs, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_no_cached_task_shows_not_found() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-999".to_string());
        app.task_detail_cache = None;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("not found") || cell_str.contains("Not found"),
            "detail view should show not found when task is missing, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_workflow() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("default"),
            "detail view should show workflow, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_detail_shows_task_id_in_header() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("TSK-001"),
            "detail view should show task ID in header, got: {cell_str}"
        );
    }

    // ── Autocomplete modal tests ──────────────────────────────────────────

    #[test]
    fn test_render_autocomplete_modal_shows_commands() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("/help"),
            "autocomplete should show /help, got: {cell_str}"
        );
        assert!(
            cell_str.contains("/clear"),
            "autocomplete should show /clear, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Slash Commands"),
            "autocomplete should show title"
        );
    }

    #[test]
    fn test_render_autocomplete_modal_filtered_shows_subset() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/gu".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("/guardrails:status"),
            "autocomplete should show /guardrails:status (buffer len={})",
            cell_str.len()
        );
        // The status bar shows "/help" as a key binding hint, so we can't
        // check !cell_str.contains("/help"). Instead verify that the modal
        // does NOT show the command entry pattern for /help — check that
        // /guardrails:status appears BEFORE /help in the buffer (modal renders
        // above status bar).
        let guard_pos = cell_str.find("/guardrails:status");
        let help_pos = cell_str.rfind("/help");
        if let (Some(g), Some(h)) = (guard_pos, help_pos) {
            // /guardrails:status (from modal) should appear before /help (from status bar)
            assert!(
                g < h,
                "filtered commands should be rendered above the status bar"
            );
        }
    }

    #[test]
    fn test_render_autocomplete_modal_shows_navigation_hints() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("navigate"),
            "autocomplete should show navigation hints (buffer len={})",
            cell_str.len()
        );
    }

    #[test]
    fn test_render_autocomplete_modal_not_visible_when_disabled() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            !cell_str.contains("Slash Commands"),
            "autocomplete should not render when not visible"
        );
    }

    #[test]
    fn test_render_autocomplete_modal_scrolls_to_follow_selection() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        // Navigate to the last command (index 10 = /tasks, the 11th item).
        // With max_visible = 10, this should trigger a scroll so the
        // selected item is visible while the first item is scrolled out.
        let filtered = app.filtered_commands();
        let last_idx = filtered.len() - 1;
        app.autocomplete_selected = last_idx;

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // The selected command (/tasks) must be visible in the modal
        let last_cmd = filtered[last_idx].name;
        assert!(
            cell_str.contains(last_cmd),
            "selected command {last_cmd} should be visible after scrolling (buffer len={})",
            cell_str.len()
        );

        // The scroll hint should indicate items were scrolled out of view
        assert!(
            cell_str.contains("above"),
            "scroll hint should indicate items above the visible window (buffer len={})",
            cell_str.len()
        );
    }

    #[test]
    fn test_conversation_scroll_offset_skips_lines() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        // Add conversation entries that will appear in the output
        app.handle_event(TuiEvent::Submit("first message".into()));
        app.handle_event(TuiEvent::Done);
        app.handle_event(TuiEvent::Submit("second message".into()));
        app.handle_event(TuiEvent::Done);
        app.handle_event(TuiEvent::Submit("third message".into()));
        app.handle_event(TuiEvent::Done);

        // With auto_scroll = true (default after Submit), all messages should be visible
        // and the view should be scrolled to show the latest content
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf0 = terminal.backend().buffer();
        let text0: String = buf0.content.iter().map(|c| c.symbol()).collect();
        assert!(
            text0.contains("third message"),
            "third message should be visible with auto-scroll"
        );

        // With auto_scroll = false and scroll_offset = 2, the top 2 visual rows are skipped
        app.auto_scroll = false;
        app.scroll_offset = 2;
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf1 = terminal.backend().buffer();
        let text1: String = buf1.content.iter().map(|c| c.symbol()).collect();
        // "third message" should still be visible
        assert!(
            text1.contains("third message"),
            "third message should be visible when scrolled"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_defaults_to_true() {
        let app = App::new();
        assert!(
            app.auto_scroll,
            "auto_scroll should default to true so new messages are visible"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_enabled_on_submit() {
        let mut app = App::new();
        app.auto_scroll = false;
        app.handle_event(TuiEvent::Submit("hello".into()));
        assert!(
            app.auto_scroll,
            "auto_scroll should be re-enabled on Submit"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_enabled_on_stream_chunk() {
        let mut app = App::new();
        app.auto_scroll = false;
        app.handle_event(TuiEvent::StreamChunk("chunk".into()));
        assert!(
            app.auto_scroll,
            "auto_scroll should be re-enabled on StreamChunk"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_enabled_on_tool_call() {
        let mut app = App::new();
        app.auto_scroll = false;
        app.handle_event(TuiEvent::ToolCall {
            id: "tc1".into(),
            name: "read".into(),
            input: None,
        });
        assert!(
            app.auto_scroll,
            "auto_scroll should be re-enabled on ToolCall"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_enabled_on_tool_result() {
        let mut app = App::new();
        app.auto_scroll = false;
        app.handle_event(TuiEvent::ToolResult {
            id: "tc1".into(),
            result: hackpi_core::tools::ToolResult::Success {
                content: "ok".into(),
            },
        });
        assert!(
            app.auto_scroll,
            "auto_scroll should be re-enabled on ToolResult"
        );
    }

    #[test]
    fn test_conversation_auto_scroll_enabled_after_clear() {
        let mut app = App::new();
        app.auto_scroll = false;
        app.clear();
        assert!(
            app.auto_scroll,
            "auto_scroll should be re-enabled after clear"
        );
    }

    #[test]
    fn test_conversation_many_messages_auto_shows_latest() {
        use ratatui::backend::TestBackend;
        // Use minimum valid terminal size; content area is ~19 rows.
        // 20 messages with prefix + number + empty line = ~80 lines → overflows.
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // Add enough messages to overflow the area
        for i in 0..20 {
            app.handle_event(TuiEvent::Submit(format!("message number {i}")));
            app.handle_event(TuiEvent::Done);
        }

        // auto_scroll should be true
        assert!(app.auto_scroll);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // The latest message should be visible (not clipped)
        assert!(
            cell_str.contains("message number 19"),
            "latest message should be visible with auto-scroll, got: {cell_str}"
        );
    }

    #[test]
    fn test_conversation_manual_scroll_prevents_auto_scroll() {
        use ratatui::backend::TestBackend;
        // Minimum valid terminal size. Content area is ~19 rows.
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // Add many messages
        for i in 0..20 {
            app.handle_event(TuiEvent::Submit(format!("message number {i}")));
            app.handle_event(TuiEvent::Done);
        }

        // Simulate user scrolling up: disable auto_scroll
        app.auto_scroll = false;
        app.scroll_offset = 0; // scroll to very top

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // The first message should be visible (we're at the top)
        assert!(
            cell_str.contains("message number 0"),
            "first message should be visible when scrolled to top"
        );

        // The latest message should NOT be visible (we're at the top of 20+ messages)
        assert!(
            !cell_str.contains("message number 19"),
            "latest message should not be visible when scrolled to top"
        );
    }

    #[test]
    fn test_count_visual_lines_empty() {
        let text = Text::from(Vec::<Line>::new());
        assert_eq!(count_visual_lines(&text, 80), 0);
    }

    #[test]
    fn test_count_visual_lines_single_short_line() {
        let text = Text::from(vec![Line::from("hello")]);
        assert_eq!(count_visual_lines(&text, 80), 1);
    }

    #[test]
    fn test_count_visual_lines_wrapping() {
        // A line that's exactly 80 chars should fit in one row
        let line_80: String = "x".repeat(80);
        let text = Text::from(vec![Line::from(line_80)]);
        assert_eq!(count_visual_lines(&text, 80), 1);

        // A line that's 81 chars should wrap to 2 rows
        let line_81: String = "x".repeat(81);
        let text = Text::from(vec![Line::from(line_81)]);
        assert_eq!(count_visual_lines(&text, 80), 2);

        // A line that's 160 chars should wrap to 2 rows
        let line_160: String = "x".repeat(160);
        let text = Text::from(vec![Line::from(line_160)]);
        assert_eq!(count_visual_lines(&text, 80), 2);
    }

    #[test]
    fn test_count_visual_lines_empty_line_counts_as_one() {
        let text = Text::from(vec![Line::from("")]);
        assert_eq!(count_visual_lines(&text, 80), 1);
    }

    #[test]
    fn test_count_visual_lines_multiple_lines() {
        let text = Text::from(vec![
            Line::from("line 1"),
            Line::from(""),
            Line::from("line 3"),
        ]);
        assert_eq!(count_visual_lines(&text, 80), 3);
    }

    #[test]
    fn test_count_visual_lines_zero_width() {
        // With zero area width, each line counts as 1
        let text = Text::from(vec![Line::from("hello"), Line::from("world")]);
        assert_eq!(count_visual_lines(&text, 0), 2);
    }

    #[test]
    fn test_render_task_detail_shows_contextual_commands() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let task = make_detail_task();
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        app.task_detail_cache = Some(task);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("/task"),
            "detail view should show contextual commands, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_board_empty_state_mentions_actions() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("No tasks yet"),
            "empty state should mention 'No tasks yet', got: {cell_str}"
        );
        assert!(
            cell_str.contains("'n'"),
            "empty state should mention 'n' key, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_board_footer_shows_n_key() {
        use ratatui::backend::TestBackend;
        // Use a wide terminal so all footer bindings fit without truncation
        let backend = TestBackend::new(160, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.task_list_cache = vec![hackpi_tasks::Task {
            id: "TSK-001".to_string(),
            title: "Test".to_string(),
            description: String::new(),
            state: "todo".to_string(),
            priority: hackpi_tasks::TaskPriority::None,
            workflow: "default".to_string(),
            blocked_by: vec![],
            labels: vec![],
            assignee: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Create task"),
            "footer should mention Create task, got tail: ...{}",
            &cell_str[cell_str.len().saturating_sub(200)..]
        );
        assert!(
            cell_str.contains("Navigate tasks"),
            "footer should mention Navigate tasks"
        );
    }

    #[test]
    fn test_render_task_create_prompt_shows_input() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskBoard;
        app.creating_task = true;
        app.task_create_input = "My new task".to_string();

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("New Task"),
            "should show 'New Task' prompt label, got: {cell_str}"
        );
        assert!(
            cell_str.contains("My new task"),
            "should show task input text, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Enter to create"),
            "should show hint, got: {cell_str}"
        );
    }

    /// Regression test for COR-160: Long command names (like /guardrails:onboarding)
    /// must not overlap with the description text. The modal must be wide enough
    /// to accommodate the longest command name plus its description without overlap.
    #[test]
    fn test_autocomplete_long_command_name_no_overlap() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/guardrails:onboarding".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // The full command name must appear
        assert!(
            cell_str.contains("/guardrails:onboarding"),
            "autocomplete should show the full command name, got: {cell_str}"
        );

        // The description must also appear
        assert!(
            cell_str.contains("Write a preset guardrails config"),
            "autocomplete should show the description for /guardrails:onboarding, got: {cell_str}"
        );
    }

    /// Regression test for COR-160: When filtering shows commands with long names,
    /// the name column width must adapt to the longest filtered command name.
    #[test]
    fn test_autocomplete_name_column_adapts_to_widest_command() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        // Filter to just guardrails commands — the longest is /guardrails:onboarding (22 chars)
        let mut app = App::new();
        app.input = "/gu".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();

        // All three guardrails commands should be visible with their descriptions
        assert!(
            cell_str.contains("/guardrails:status"),
            "should show /guardrails:status, got: {cell_str}"
        );
        assert!(
            cell_str.contains("/guardrails:clean"),
            "should show /guardrails:clean, got: {cell_str}"
        );
        assert!(
            cell_str.contains("/guardrails:onboarding"),
            "should show /guardrails:onboarding, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Show guardrails status"),
            "should show description for status, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Clear session cache"),
            "should show description for clean, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Write a preset guardrails config"),
            "should show description for onboarding, got: {cell_str}"
        );
    }

    /// Regression test for COR-158: After submitting a message, the submitted
    /// text must NOT appear in the input area (which would create a ghost
    /// textbox above the real one).
    #[test]
    fn test_no_ghost_textbox_after_message_submit() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // Simulate the user having typed "hello" into the input,
        // then pressing Enter (which clears the buffer and submits).
        // The Submit handler now clears app.input.
        app.input = "hello".to_string();
        app.handle_event(TuiEvent::Submit("hello".into()));

        // app.input should be empty (Submit clears it)
        assert!(
            app.input.is_empty(),
            "app.input should be empty after Submit"
        );

        // Render after submit
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let width = 80u16;

        // Extract the conversation area rows (rows 1-19) and input area rows (20-22)
        let input_row_start = 20usize;
        let input_row_end = 22usize;
        let input_rows: String = (input_row_start..=input_row_end)
            .map(|row| {
                let start = row * (width as usize);
                let end = start + (width as usize);
                buffer.content[start..end]
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<Vec<&str>>()
                    .join("")
            })
            .collect::<Vec<String>>()
            .join("\n");

        // The input area must NOT contain the submitted text "hello"
        assert!(
            !input_rows.contains("hello"),
            "input area should NOT contain 'hello' after submit (ghost textbox). \
             Input rows: {input_rows}"
        );

        // The input area should contain the "> " prompt (empty input during Generating)
        assert!(
            input_rows.contains("> "),
            "input area should contain '> ' prompt. Input rows: {input_rows}"
        );

        // Full buffer check: conversation should show the user message
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("○ me: hello"),
            "conversation should show user message, got: {cell_str}"
        );
    }

    // ── Input cursor visibility tests (COR-162) ──────────────────────────────

    /// Regression test for COR-162: The terminal cursor must be visible and
    /// positioned at the current typing position when the app is in Resting state.
    #[test]
    fn test_input_cursor_visible_at_typing_position() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "hello".to_string();
        app.input_cursor = 3; // cursor between "hel" and "lo"

        terminal.draw(|f| render(f, &app)).unwrap();

        // Cursor should be positioned at:
        //   x = input_area.x + prefix_len ("> " = 2) + cursor_offset (3) = 5
        //   y = input_area.y (first row of input inner area)
        // Layout: row 0 = tab header, rows 1-19 = content, rows 20-22 = input block.
        // input_block has Borders::TOP, so inner area starts at row 21.
        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(
            pos.x, 5,
            "cursor x should be at col 5 (prefix 2 + cursor offset 3), got {}",
            pos.x
        );
        assert_eq!(
            pos.y, 21,
            "cursor y should be at input inner area row 21, got {}",
            pos.y
        );
    }

    /// When the input is empty, the cursor should still be visible right after
    /// the "> " prefix in Resting state.
    #[test]
    fn test_input_cursor_visible_when_empty() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        // Default: input is empty, cursor at 0, state is Resting

        terminal.draw(|f| render(f, &app)).unwrap();

        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(
            pos.x, 2,
            "cursor x should be at col 2 (after '> ' prefix), got {}",
            pos.x
        );
        assert_eq!(
            pos.y, 21,
            "cursor y should be at input inner area row 21, got {}",
            pos.y
        );
    }

    /// Cursor should be at the end of typed text when cursor == input.len().
    #[test]
    fn test_input_cursor_at_end_of_text() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "test".to_string();
        app.input_cursor = 4; // at end

        terminal.draw(|f| render(f, &app)).unwrap();

        let pos = terminal.get_cursor_position().unwrap();
        assert_eq!(
            pos.x, 6,
            "cursor x should be at col 6 (prefix 2 + text len 4), got {}",
            pos.x
        );
    }

    // ── COR-277: Terminal size gate tests ────────────────────────────────────

    #[test]
    fn test_is_too_small_returns_true_for_small_terminal() {
        let area_60x20 = Rect::new(0, 0, 60, 20);
        assert!(
            is_too_small(area_60x20),
            "60x20 should be too small (width < 80)"
        );

        let area_80x20 = Rect::new(0, 0, 80, 20);
        assert!(
            is_too_small(area_80x20),
            "80x20 should be too small (height < 24)"
        );

        let area_60x24 = Rect::new(0, 0, 60, 24);
        assert!(
            is_too_small(area_60x24),
            "60x24 should be too small (width < 80)"
        );
    }

    #[test]
    fn test_is_too_small_returns_false_for_minimum_terminal() {
        let area_80x24 = Rect::new(0, 0, 80, 24);
        assert!(!is_too_small(area_80x24), "80x24 should be large enough");
    }

    #[test]
    fn test_is_too_small_returns_false_for_large_terminal() {
        let area_120x40 = Rect::new(0, 0, 120, 40);
        assert!(!is_too_small(area_120x40), "120x40 should be large enough");

        let area_200x60 = Rect::new(0, 0, 200, 60);
        assert!(!is_too_small(area_200x60), "200x60 should be large enough");
    }

    #[test]
    fn test_render_too_small_shows_resize_message() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(60, 20);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Terminal too small"),
            "should show resize message on small terminal, got: {cell_str}"
        );
        assert!(
            cell_str.contains("80x24"),
            "should show minimum dimensions, got: {cell_str}"
        );
        assert!(
            cell_str.contains("60x20"),
            "should show current dimensions, got: {cell_str}"
        );
        // Normal UI content should NOT appear
        assert!(
            !cell_str.contains("Conversation"),
            "should NOT render tabs when terminal is too small"
        );
    }

    #[test]
    fn test_render_at_80x24_shows_normal_ui() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Conv"),
            "80x24 should render tabs: {cell_str}"
        );
        assert!(
            cell_str.contains("Type a message"),
            "80x24 should render input placeholder: {cell_str}"
        );
        assert!(
            !cell_str.contains("Terminal too small"),
            "80x24 should NOT show resize message"
        );
    }

    #[test]
    fn test_render_at_120x40_shows_normal_ui() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Conv"),
            "120x40 should render tabs: {cell_str}"
        );
        assert!(
            !cell_str.contains("Terminal too small"),
            "120x40 should NOT show resize message"
        );
    }

    #[test]
    fn test_render_at_200x60_shows_normal_ui() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(200, 60);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Conv"),
            "200x60 should render tabs: {cell_str}"
        );
        assert!(
            !cell_str.contains("Terminal too small"),
            "200x60 should NOT show resize message"
        );
    }

    // ── COR-277: Responsive modal helper tests ───────────────────────────────

    #[test]
    fn test_centered_rect_fits_within_area() {
        let area = Rect::new(0, 0, 100, 50);
        let centered = centered_rect(area, 50, 50);

        // Centered rect should be fully inside the parent area
        assert!(
            centered.x >= area.x,
            "centered x={} should be >= area.x={}",
            centered.x,
            area.x
        );
        assert!(
            centered.y >= area.y,
            "centered y={} should be >= area.y={}",
            centered.y,
            area.y
        );
        assert!(
            centered.right() <= area.right(),
            "centered right={} should be <= area.right={}",
            centered.right(),
            area.right()
        );
        assert!(
            centered.bottom() <= area.bottom(),
            "centered bottom={} should be <= area.bottom={}",
            centered.bottom(),
            area.bottom()
        );

        // With 50% of 100 = 50
        assert_eq!(centered.width, 50, "50% of 100 width should be 50");
        assert_eq!(centered.height, 25, "50% of 50 height should be 25");
        assert_eq!(centered.x, 25, "centered x should be 25");
        assert_eq!(centered.y, 12, "centered y should be 12 (integer division)");
    }

    #[test]
    fn test_centered_rect_100_percent_is_full_area() {
        let area = Rect::new(0, 0, 80, 24);
        let centered = centered_rect(area, 100, 100);
        assert_eq!(centered, area, "100% centered rect should equal area");
    }

    #[test]
    fn test_modal_rect_respects_preferred_size() {
        // On a large terminal, the preferred size should be used
        let area = Rect::new(0, 0, 200, 80);
        let modal = modal_rect(area, 60, 15, 70, 70);
        assert_eq!(
            modal.width, 60,
            "on large terminal, should use preferred width of 60"
        );
        assert_eq!(
            modal.height, 15,
            "on large terminal, should use preferred height of 15"
        );
        assert!(modal.x > 0, "modal should be centered (x={})", modal.x);
        assert!(modal.y > 0, "modal should be centered (y={})", modal.y);
    }

    #[test]
    fn test_modal_rect_scales_down_on_small_terminal() {
        // On a small terminal where 70% is less than preferred, should use percentage
        let area = Rect::new(0, 0, 50, 20);
        let modal = modal_rect(area, 60, 15, 70, 70);
        // 70% of 50 = 35, which is < 60
        assert_eq!(
            modal.width, 35,
            "on narrow terminal, width should be 70% of area"
        );
        // 70% of 20 = 14, which is < 15
        assert_eq!(
            modal.height, 14,
            "on short terminal, height should be 70% of area"
        );
    }

    #[test]
    fn test_modal_rect_never_exceeds_area() {
        let area = Rect::new(0, 0, 10, 5);
        let modal = modal_rect(area, 60, 15, 100, 100);
        assert!(
            modal.width <= area.width,
            "width={} should not exceed area width={}",
            modal.width,
            area.width
        );
        assert!(
            modal.height <= area.height,
            "height={} should not exceed area height={}",
            modal.height,
            area.height
        );
    }

    // ── COR-277: Permission modal with long field values tests ───────────────

    #[test]
    fn test_truncate_for_display_short_string() {
        assert_eq!(truncate_for_display("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_for_display_exact_fit() {
        assert_eq!(truncate_for_display("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_for_display_long_string() {
        let result = truncate_for_display("a very long string that should be truncated", 10);
        assert_eq!(
            result.chars().count(),
            10,
            "truncated string should be 10 chars"
        );
        assert!(
            result.ends_with('…'),
            "truncated string should end with …: {result}"
        );
    }

    #[test]
    fn test_truncate_for_display_empty_string() {
        assert_eq!(truncate_for_display("", 10), "");
    }

    #[test]
    fn test_truncate_for_display_zero_max() {
        let result = truncate_for_display("hello", 0);
        assert_eq!(result, "…", "zero max should just return …");
    }

    #[test]
    fn test_truncate_for_display_one_char_max() {
        let result = truncate_for_display("hello", 1);
        assert_eq!(result, "…", "1 char max should return …");
    }

    // ── COR-277: Autocomplete bounds at narrow widths tests ───────────────────

    #[test]
    fn test_render_autocomplete_at_narrow_width_does_not_panic() {
        use ratatui::backend::TestBackend;
        // Even at 80x24 minimum, autocomplete should not panic
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        // Should not panic
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    #[test]
    fn test_render_autocomplete_at_very_narrow_width_does_not_panic() {
        use ratatui::backend::TestBackend;
        // 81x24 - just above minimum, autocomplete should not cause overflow panics
        let backend = TestBackend::new(81, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.input = "/".to_string();
        app.update_autocomplete_state();
        assert!(app.autocomplete_visible);

        // Should not panic
        terminal.draw(|f| render(f, &app)).unwrap();
    }

    // ── COR-277: split_root layout tests ─────────────────────────────────────

    #[test]
    fn test_split_root_returns_correct_regions() {
        let area = Rect::new(0, 0, 80, 24);
        let root = split_root(area);

        // Header should be 1 row
        assert_eq!(root.header.height, 1);
        assert_eq!(root.header.x, 0);
        assert_eq!(root.header.y, 0);

        // Status should be last row
        assert_eq!(root.status.height, 1);
        assert_eq!(root.status.y, 23);

        // Input should be 3 rows
        assert_eq!(root.input.height, 3);

        // Main should fill remaining (24 - 1 - 3 - 1 = 19)
        assert_eq!(root.main.height, 19);

        // All regions should have the full width
        assert_eq!(root.header.width, 80);
        assert_eq!(root.main.width, 80);
        assert_eq!(root.input.width, 80);
        assert_eq!(root.status.width, 80);
    }

    // ── COR-277: Status bar does not wrap tests ──────────────────────────────

    #[test]
    fn test_render_status_no_wrap_at_narrow_width() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|f| render(f, &app)).unwrap();

        // Status bar is at row 23 (0-indexed, last row)
        let buffer = terminal.backend().buffer();
        let row_23_start = 23 * 80;
        let status_row: String = buffer.content[row_23_start..row_23_start + 80]
            .iter()
            .map(|c| c.symbol())
            .collect();

        // The status bar should contain binding hints
        assert!(
            status_row.contains("Interrupt generation"),
            "status bar should show bindings, got: {status_row:?}"
        );

        // The status bar should NOT contain newlines (indicates wrapping)
        assert!(
            !status_row.contains('\n'),
            "status bar should not have newlines"
        );
    }

    // ── COR-277: Help overlay responsive sizing tests ────────────────────────

    #[test]
    fn test_render_help_overlay_uses_responsive_sizing() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.help_visible = true;

        terminal.draw(|f| render(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("Help"),
            "help overlay should show, got: {cell_str}"
        );
        assert!(
            cell_str.contains("Ctrl+C"),
            "help overlay should show Ctrl+C"
        );
        assert!(
            cell_str.contains("Ctrl+D"),
            "help overlay should show Ctrl+D"
        );
    }

    // ── COR-278: Unicode-aware text measurement tests ──────────────────────────

    #[test]
    fn test_display_width_prefix_ascii() {
        assert_eq!(display_width_prefix("hello", 0), 0);
        assert_eq!(display_width_prefix("hello", 3), 3);
        assert_eq!(display_width_prefix("hello", 5), 5);
        assert_eq!(display_width_prefix("", 0), 0);
    }

    #[test]
    fn test_display_width_prefix_cjk() {
        // CJK characters are 2 columns wide
        assert_eq!(display_width_prefix("世界", 0), 0);
        assert_eq!(display_width_prefix("世界", 1), 2); // "世" = 2
        assert_eq!(display_width_prefix("世界", 2), 4); // "世界" = 4
    }

    #[test]
    fn test_display_width_prefix_emoji() {
        // Many emoji are 2 columns wide
        assert_eq!(display_width_prefix("a🔥b", 0), 0);
        assert_eq!(display_width_prefix("a🔥b", 1), 1); // "a" = 1
        assert_eq!(display_width_prefix("a🔥b", 2), 3); // "a🔥" = 1 + 2 = 3
        assert_eq!(display_width_prefix("a🔥b", 3), 4); // "a🔥b" = 1 + 2 + 1 = 4
    }

    #[test]
    fn test_display_width_prefix_mixed() {
        // Mixed ASCII + CJK + ASCII
        assert_eq!(display_width_prefix("ab中cd", 2), 2); // "ab" = 2
        assert_eq!(display_width_prefix("ab中cd", 3), 4); // "ab中" = 2 + 2 = 4
        assert_eq!(display_width_prefix("ab中cd", 4), 5); // "ab中c" = 2 + 2 + 1 = 5
        assert_eq!(display_width_prefix("ab中cd", 5), 6); // "ab中cd" = 2 + 2 + 2 = 6
    }

    #[test]
    fn test_display_width_prefix_past_end() {
        // Cursor past the end should give the full width
        assert_eq!(display_width_prefix("hi", 10), 2);
        assert_eq!(display_width_prefix("世界", 10), 4);
    }

    // ── cursor_position_for_input tests ──────────────────────────────────

    #[test]
    fn test_cursor_position_single_line_ascii() {
        let input_area_width = 80;
        let prefix_width = 2;

        // Empty input
        let (row, col) = cursor_position_for_input("", 0, input_area_width, prefix_width);
        assert_eq!(row, 0, "empty input row");
        assert_eq!(col, 2, "empty input col (after prefix)");

        // "hello", cursor at start
        let (row, col) = cursor_position_for_input("hello", 0, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 2);

        // "hello", cursor at end
        let (row, col) = cursor_position_for_input("hello", 5, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 7); // prefix(2) + "hello"(5)
    }

    #[test]
    fn test_cursor_position_single_line_cjk() {
        let input_area_width = 80;
        let prefix_width = 2;

        // "世界", cursor at 0
        let (row, col) = cursor_position_for_input("世界", 0, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 2, "cursor before first CJK char");

        // "世界", cursor at 1 (after first CJK char)
        let (row, col) = cursor_position_for_input("世界", 1, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 4, "cursor after '世' (width 2 + 2 = 4)");

        // "世界", cursor at 2 (after both)
        let (row, col) = cursor_position_for_input("世界", 2, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 6, "cursor after '世界' (width 2 + 4 = 6)");
    }

    #[test]
    fn test_cursor_position_multiline_ascii() {
        let input_area_width = 80;
        let prefix_width = 2;

        // "hello\nworld", cursor at start of second line
        let (row, col) =
            cursor_position_for_input("hello\nworld", 6, input_area_width, prefix_width);
        assert_eq!(row, 1, "second line");
        assert_eq!(col, 0, "start of second line");

        // "hello\nworld", cursor at end
        let (row, col) =
            cursor_position_for_input("hello\nworld", 11, input_area_width, prefix_width);
        assert_eq!(row, 1, "second line");
        assert_eq!(col, 5, "end of second line");
    }

    #[test]
    fn test_cursor_position_multiline_cjk() {
        let input_area_width = 80;
        let prefix_width = 2;

        // "你好\n世界", cursor at start of second line (char index 2 = \n...)
        // Wait: "你好\n世界" chars: 你(0), 好(1), \n(2), 世(3), 界(4)
        let (row, col) = cursor_position_for_input("你好\n世界", 3, input_area_width, prefix_width);
        assert_eq!(row, 1, "second line");
        assert_eq!(col, 0, "start of second line after \\n");

        // "你好\n世界", cursor at end
        let (row, col) = cursor_position_for_input("你好\n世界", 5, input_area_width, prefix_width);
        assert_eq!(row, 1, "second line");
        assert_eq!(col, 4, "end of '世界' = 2+2");
    }

    #[test]
    fn test_cursor_position_wrapping() {
        let input_area_width = 10;
        let prefix_width = 2;

        // Text that's exactly one wrapped line
        // "hello" has display width 5, with prefix 2 = 7. Fits in 10.
        let (row, col) = cursor_position_for_input("hello", 5, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 7);

        // A long string that wraps: "1234567890" + prefix = 12 width in 10-wide area
        // Row 0: "> " + "12345678" (10 cols)
        // Row 1: "90" (2 cols)
        let long = "1234567890";
        let (row, col) = cursor_position_for_input(long, 10, input_area_width, prefix_width);
        assert_eq!(row, 1, "should wrap to second visual line");
        assert_eq!(col, 2, "column after wrapping");

        // Cursor partway through
        // Row 0: "> " + "12345678" (10 cols)
        // Cursor at position 5 (after "12345")
        // Display width = 2 + 5 = 7, fits on first row
        let (row, col) = cursor_position_for_input(long, 5, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 7, "prefix(2) + 5 chars = 7");
    }

    #[test]
    fn test_cursor_position_wrapping_cjk() {
        let input_area_width = 8;
        let prefix_width = 2;

        // Each CJK char is 2 wide
        // "> 一二三四" has display width 2+8=10. With area width 8:
        // Row 0: "> 一二三" = 2+2+2+2 = 8 (exactly fills row)
        // Row 1: "四" = 2
        let text = "一二三四";
        let (row, col) = cursor_position_for_input(text, 4, input_area_width, prefix_width);
        assert_eq!(row, 1, "should wrap to second visual line");
        assert_eq!(col, 2, "第四  has display width 2");

        // Cursor at position 2 (after 一二)
        // Display width = 2 + 4 = 6, fits on row 0
        let (row, col) = cursor_position_for_input(text, 2, input_area_width, prefix_width);
        assert_eq!(row, 0);
        assert_eq!(col, 6, "prefix(2) + 一二(4) = 6");
    }

    #[test]
    fn test_cursor_position_zero_width_area() {
        let (row, col) = cursor_position_for_input("hello", 3, 0, 2);
        assert_eq!(row, 0, "zero width area, row should be 0");
        assert_eq!(col, 2, "zero width area, col should be prefix_width");
    }

    #[test]
    fn test_cursor_position_empty_lines_before_cursor() {
        let input_area_width = 80;
        let prefix_width = 2;

        // Multiple empty lines: "a\n\n\nb", cursor at end
        // chars: a(0), \n(1), \n(2), \n(3), b(4)
        // Lines: "a", "", "", "b"
        let (row, col) = cursor_position_for_input("a\n\n\nb", 5, input_area_width, prefix_width);
        assert_eq!(row, 3, "3 newlines before cursor = 4th line");
        assert_eq!(col, 1, "b has width 1");
    }

    // ── truncate_to_width tests ──────────────────────────────────────────

    #[test]
    fn test_truncate_to_width_ascii() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello", 5), "hello");
        assert_eq!(truncate_to_width("hello world", 5), "hello");
        assert_eq!(truncate_to_width("", 10), "");
        assert_eq!(truncate_to_width("hello", 0), "");
    }

    #[test]
    fn test_truncate_to_width_cjk() {
        // Each CJK char is 2 wide
        assert_eq!(truncate_to_width("世界", 4), "世界"); // fits exactly (2 + 2 = 4)
        assert_eq!(truncate_to_width("世界", 3), "世…"); // "世" fits (2), "界" would exceed (4 > 3), "…" fits
        assert_eq!(truncate_to_width("世界", 2), "世"); // exactly 1 CJK char fits
        assert_eq!(truncate_to_width("世界", 1), "…"); // "世" (2) > 1, no char fits, but "…" fits
                                                       // "世" width is 2, 0 + 2 = 2 > 1. truncated = true. current_width is 0.
                                                       // 0 + 1 <= 1, so we push '…'.
                                                       // Actually, hmm. This seems odd. If we can't even fit the first character, should we still show '…'?
                                                       // With max_width=1, result would be "…".
        assert_eq!(truncate_to_width("世界", 1), "…");
    }

    #[test]
    fn test_truncate_to_width_mixed() {
        // "a世b" has widths: 1 + 2 + 1 = 4
        assert_eq!(truncate_to_width("a世b", 4), "a世b");
        assert_eq!(truncate_to_width("a世b", 3), "a世"); // a(1)+世(2)=3 fits, then 'b'(1) makes 4>3
        assert_eq!(truncate_to_width("a世b", 2), "a…");
        assert_eq!(truncate_to_width("a世b", 1), "a");
    }

    #[test]
    fn test_truncate_to_width_emoji() {
        // fire emoji is 2 wide
        assert_eq!(truncate_to_width("a🔥b", 4), "a🔥b"); // 1+2+1=4
        assert_eq!(truncate_to_width("a🔥b", 3), "a🔥"); // 1+2=3, b would make 4
        assert_eq!(truncate_to_width("a🔥b", 2), "a…");
    }
}
