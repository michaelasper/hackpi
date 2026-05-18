use crate::app::{AppState, AppView, ToolCallStatus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab header
            Constraint::Min(1),    // main content
            Constraint::Length(3), // input
            Constraint::Length(1), // status bar
        ])
        .split(area);

    render_tab_header(frame, chunks[0], &app.active_view, app);

    match &app.active_view {
        AppView::Conversation => {
            render_conversation(frame, chunks[1], app);
        }
        AppView::TaskDetail(_) => {
            render_task_detail(frame, chunks[1], app);
        }
        AppView::TaskBoard => {
            render_task_board(frame, chunks[1], app);
        }
        AppView::TaskGraph => {
            render_placeholder(frame, chunks[1], "Graph view coming soon...");
        }
    }

    render_input(frame, chunks[2], app);
    render_status(frame, chunks[3], app);

    if app.autocomplete_visible {
        render_autocomplete_modal(frame, chunks[2], app);
    }

    if app.pending_permission.is_some() {
        render_permission_modal(frame, area, app);
    }
}

/// Render the tab header with active/inactive tab highlighting and version/usage info.
fn render_tab_header(frame: &mut Frame, area: Rect, active_view: &AppView, app: &App) {
    let tabs = [
        ("Conversation", matches!(active_view, AppView::Conversation)),
        (
            "Tasks",
            matches!(active_view, AppView::TaskBoard | AppView::TaskDetail(_)),
        ),
        ("Graph", matches!(active_view, AppView::TaskGraph)),
    ];

    let mut spans: Vec<Span> = Vec::new();
    for (i, (label, is_active)) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("    "));
        }
        spans.push(Span::styled(
            format!("[Tab] {label}"),
            if *is_active {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else if *label == "Graph" {
                // Graph is a future placeholder — dimmed
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::DarkGray)
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
        Style::default().fg(Color::DarkGray),
    ));

    let line = Line::from(spans);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn tool_card_color(name: &str) -> Color {
    match name {
        "read" => Color::Blue,
        "edit" => Color::Magenta,
        "bash" => Color::Yellow,
        "search_grep" => Color::Cyan,
        "write" => Color::Green,
        "git_read" => Color::Rgb(100, 180, 100), // green — read-only, safe
        "git_write" => Color::Rgb(255, 140, 0),  // orange — mutation, caution
        "github" => Color::Rgb(255, 255, 255),   // white — external API
        _ => Color::DarkGray,
    }
}

fn user_prefix() -> &'static str {
    " ○ me: "
}

fn assistant_prefix() -> &'static str {
    " ● assistant: "
}

fn render_conversation(frame: &mut Frame, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.conversation {
        let prefix = match entry.role.as_str() {
            "user" => user_prefix(),
            "assistant" => assistant_prefix(),
            _ => "   ",
        };

        let role_style = match entry.role.as_str() {
            "user" => Style::default().fg(Color::Green),
            "assistant" => Style::default().fg(Color::Cyan),
            _ => Style::default(),
        };

        if !entry.text.is_empty() {
            let content = format!("{prefix}{}", entry.text);
            lines.push(Line::from(Span::styled(content, role_style)));
        }

        for tc in &entry.tool_calls {
            let border_color = tool_card_color(&tc.name);
            let (status_symbol, _status_color) = match &tc.status {
                ToolCallStatus::Running => ("⋯", Color::Yellow),
                ToolCallStatus::Done(result) => match result {
                    hackpi_core::tools::ToolResult::Success { .. } => ("✓", Color::Green),
                    hackpi_core::tools::ToolResult::SystemError { .. } => ("✗", Color::Red),
                    hackpi_core::tools::ToolResult::Timeout => ("⚠", Color::Yellow),
                    hackpi_core::tools::ToolResult::Cancelled => ("⊘", Color::Gray),
                },
            };

            let title = format!(" {status_symbol} {name} ", name = tc.name);

            // Push title BEFORE result content so it appears right above, not at the top
            lines.push(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(border_color)
                    .add_modifier(Modifier::BOLD),
            )));

            if let ToolCallStatus::Done(result) = &tc.status {
                let result_content = match result {
                    hackpi_core::tools::ToolResult::Success { content } => content.clone(),
                    hackpi_core::tools::ToolResult::SystemError { message } => {
                        format!("Error: {message}")
                    }
                    hackpi_core::tools::ToolResult::Timeout => "Timed out.".into(),
                    hackpi_core::tools::ToolResult::Cancelled => "Cancelled.".into(),
                };
                for line_content in result_content.lines() {
                    lines.push(Line::from(Span::raw(line_content.to_string())));
                }
            } else {
                lines.push(Line::from(Span::styled(
                    "Running...",
                    Style::default().fg(Color::Yellow),
                )));
            }
        }

        lines.push(Line::from(""));
    }

    let visible_lines: &[Line] = if app.scroll_offset > 0 && app.scroll_offset < lines.len() {
        &lines[app.scroll_offset..]
    } else {
        &lines
    };

    let paragraph = Paragraph::new(Text::from(visible_lines.to_vec()))
        .block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Color for a task state badge.
fn task_state_color(state: &str) -> Color {
    match state {
        "todo" => Color::Gray,
        "in_progress" => Color::Yellow,
        "blocked" => Color::Red,
        "in_review" => Color::Blue,
        "done" => Color::Green,
        "cancelled" => Color::DarkGray,
        _ => Color::DarkGray,
    }
}

/// Render the task board list view.
fn render_task_board(frame: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    if app.task_list_cache.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No tasks. Use /task create <title> to add one.",
            Style::default().fg(Color::DarkGray),
        ))));
    } else {
        for (i, task) in app.task_list_cache.iter().enumerate() {
            let is_selected = i == app.selected_task_idx;
            let state_color = task_state_color(&task.state);
            let cursor = if is_selected { "▸ " } else { "  " };

            // Main task line: TSK-001 [in_progress] Implement auth module
            let mut line_spans: Vec<Span> = Vec::new();

            // Selection cursor
            line_spans.push(Span::styled(
                cursor.to_string(),
                if is_selected {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                },
            ));

            // Task ID
            line_spans.push(Span::styled(
                format!("{} ", task.id),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));

            // State badge
            line_spans.push(Span::styled(
                format!("[{}] ", task.state),
                Style::default().fg(state_color),
            ));

            // Title
            line_spans.push(Span::styled(
                task.title.clone(),
                if is_selected {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::Reset)
                },
            ));

            items.push(ListItem::new(Line::from(line_spans)));

            // Show blocked_by as indented sub-entries
            if !task.blocked_by.is_empty() {
                for blocker_id in &task.blocked_by {
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!("      ⬑ blocked by {}", blocker_id),
                        Style::default().fg(Color::Red),
                    ))));
                }
            }
        }
    }

    // Footer showing available commands
    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(Span::styled(
        "  ↑/↓ navigate  Enter detail  Esc back  /task create <title>  /task move <id> <state>",
        Style::default().fg(Color::DarkGray),
    ))));

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default()),
    );

    frame.render_widget(list, area);
}

/// Render a placeholder view for unimplemented tabs.
fn render_placeholder(frame: &mut Frame, area: Rect, message: &str) {
    let text = Paragraph::new(message)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(text, area);
}

/// Format a DateTime<Utc> as a local time string (YYYY-MM-DD HH:MM).
fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*dt);
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Render the task detail view showing full task information.
fn render_task_detail(frame: &mut Frame, area: Rect, app: &App) {
    let task = match &app.task_detail_cache {
        Some(t) => t,
        None => {
            let text = Paragraph::new("Task not found")
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center);
            frame.render_widget(text, area);
            return;
        }
    };

    let id = match &app.active_view {
        AppView::TaskDetail(id) => id.clone(),
        _ => task.id.clone(),
    };

    let em_dash = "—";

    // Build the detail lines
    let mut lines: Vec<Line> = Vec::new();

    // Title bar
    lines.push(Line::from(Span::styled(
        format!(" Task: {} ", id),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Title field
    lines.push(Line::from(vec![
        Span::styled("  Title:       ", Style::default().fg(Color::DarkGray)),
        Span::styled(task.title.clone(), Style::default().fg(Color::White)),
    ]));

    // State field (colored)
    let state_color = task_state_color(&task.state);
    lines.push(Line::from(vec![
        Span::styled("  State:       ", Style::default().fg(Color::DarkGray)),
        Span::styled(task.state.clone(), Style::default().fg(state_color)),
    ]));

    // Priority field
    let priority_str = format!("{:?}", task.priority).to_lowercase();
    lines.push(Line::from(vec![
        Span::styled("  Priority:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(priority_str, Style::default().fg(Color::White)),
    ]));

    // Workflow field
    lines.push(Line::from(vec![
        Span::styled("  Workflow:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(task.workflow.clone(), Style::default().fg(Color::White)),
    ]));

    // Created field
    lines.push(Line::from(vec![
        Span::styled("  Created:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_timestamp(&task.created_at),
            Style::default().fg(Color::White),
        ),
    ]));

    // Updated field
    lines.push(Line::from(vec![
        Span::styled("  Updated:     ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_timestamp(&task.updated_at),
            Style::default().fg(Color::White),
        ),
    ]));

    // Assignee field
    let assignee_display = task.assignee.as_deref().unwrap_or(em_dash);
    lines.push(Line::from(vec![
        Span::styled("  Assignee:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            assignee_display.to_string(),
            Style::default().fg(Color::White),
        ),
    ]));

    // Labels field
    let labels_display = if task.labels.is_empty() {
        em_dash.to_string()
    } else {
        task.labels.join(", ")
    };
    lines.push(Line::from(vec![
        Span::styled("  Labels:      ", Style::default().fg(Color::DarkGray)),
        Span::styled(labels_display, Style::default().fg(Color::White)),
    ]));

    // Blocked by field
    let blocked_by_display = if app.task_detail_blocked_by.is_empty() {
        em_dash.to_string()
    } else {
        app.task_detail_blocked_by
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    lines.push(Line::from(vec![
        Span::styled("  Blocked by:  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            blocked_by_display,
            if app.task_detail_blocked_by.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Red)
            },
        ),
    ]));

    // Blocking field
    let blocking_display = if app.task_detail_blocking.is_empty() {
        em_dash.to_string()
    } else {
        app.task_detail_blocking
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>()
            .join(", ")
    };
    lines.push(Line::from(vec![
        Span::styled("  Blocking:    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            blocking_display,
            if app.task_detail_blocking.is_empty() {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ),
    ]));

    lines.push(Line::from(""));

    // Description section
    lines.push(Line::from(Span::styled(
        "  Description:",
        Style::default().fg(Color::DarkGray),
    )));
    if task.description.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {em_dash}"),
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for desc_line in task.description.lines() {
            lines.push(Line::from(Span::raw(format!("  {desc_line}"))));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  /task move {id} done  or  /task block <id> {id}"),
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default());

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let input_block = Block::default()
        .borders(Borders::TOP)
        .style(
            Style::default().fg(if matches!(app.state, AppState::Generating) {
                Color::DarkGray
            } else {
                Color::White
            }),
        );

    let input_area = input_block.inner(area);
    frame.render_widget(input_block, area);

    let prefix = "> ";
    let display = if app.input.is_empty() && matches!(app.state, AppState::Resting) {
        format!("{prefix}type a message...")
    } else {
        format!("{prefix}{}", app.input)
    };

    let paragraph = Paragraph::new(display).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, input_area);
}

fn status_bar_text(app: &App) -> String {
    let view_hint = match &app.active_view {
        AppView::Conversation => "",
        AppView::TaskBoard => "Tab:Tasks  ",
        AppView::TaskDetail(id) => return format!(" Task: {id}  |  Esc back  ↑/↓ navigate  ●"),
        AppView::TaskGraph => "Tab:Graph  ",
    };
    let state_text = match app.state {
        AppState::Resting => "Ctrl+C interrupt  Ctrl+L clear  Ctrl+D exit  /help",
        AppState::Generating => "Generating... (Ctrl+C to interrupt)",
        AppState::Interrupted => "Interrupted. Press any key.",
    };
    let conn = "●";
    if app.status_message.is_empty() {
        format!(" {view_hint}{state_text}  {conn}")
    } else {
        format!(" {view_hint}{} | {state_text}  {conn}", app.status_message)
    }
}

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let text = status_bar_text(app);

    let style = match app.state {
        AppState::Resting => Style::default().fg(Color::DarkGray),
        AppState::Generating => Style::default().fg(Color::Yellow),
        AppState::Interrupted => Style::default().fg(Color::Red),
    };

    frame.render_widget(
        Paragraph::new(text).style(style).wrap(Wrap { trim: true }),
        area,
    );
}

/// Render the slash command autocomplete popover above the input area.
fn render_autocomplete_modal(frame: &mut Frame, input_area: Rect, app: &App) {
    let filtered = app.filtered_commands();
    if filtered.is_empty() {
        return;
    }

    // Modal dimensions — use the full terminal area frame for reference
    let frame_area = frame.area();
    let modal_width = frame_area.width.min(60);
    let max_visible = 10; // max items to show at once
    let item_count = filtered.len().min(max_visible);
    // Height: top border (1) + title (1) + items + optional "more" (1) + hint (1) + bottom border (1)
    let modal_height = (item_count + 5).min(16) as u16;

    // Position above the input area, indented from the left, clamped to top of screen
    let x = input_area.x + 2;
    let y = input_area.y.saturating_sub(modal_height).max(1); // leave room for tab header

    let modal_area = Rect {
        x,
        y,
        width: modal_width,
        height: modal_height,
    };

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    // Build list lines
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " Slash Commands ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    // Command items
    let display_count: usize = filtered.len().min(max_visible);
    for (i, cmd) in filtered.iter().enumerate().take(display_count) {
        let is_selected = i == app.autocomplete_selected;
        let cursor = if is_selected { "▸ " } else { "  " };

        let cmd_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let desc_style = if is_selected {
            Style::default().fg(Color::Gray).bg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Gray)
        };

        lines.push(Line::from(vec![
            Span::styled(cursor.to_string(), cmd_style),
            Span::styled(format!("{:<20}", cmd.name), cmd_style),
            Span::styled(cmd.description, desc_style),
        ]));
    }

    // Hint line
    if filtered.len() > max_visible {
        lines.push(Line::from(Span::styled(
            format!("  … and {} more", filtered.len() - max_visible),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(Span::styled(
        "  ↑/↓ navigate  Tab select  Enter submit  Esc close",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}

#[allow(clippy::vec_init_then_push)]
fn render_permission_modal(frame: &mut Frame, area: Rect, app: &App) {
    let prompt = match &app.pending_permission {
        Some(p) => p,
        None => return,
    };

    // Modal dimensions: 60 chars wide, proportional height
    let modal_width = 60;
    let modal_height = 15;
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect {
        x,
        y,
        width: modal_width.min(area.width),
        height: modal_height.min(area.height),
    };

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    let reason = &prompt.reason;
    let guard_name = format!("{}", reason.guard);

    // Build modal content lines
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " ⚠ Permission Required ",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Details
    lines.push(Line::from(vec![
        Span::styled(" Pattern: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&reason.details),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" Tool: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&reason.tool),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Guard: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&guard_name),
    ]));
    lines.push(Line::from(""));

    // Options
    lines.push(Line::from(Span::styled(
        " [1] Allow once           [3] Deny",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        " [2] Allow session        [4] Always allow",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(Span::styled(
        "                         [5] Always deny",
        Style::default().fg(Color::Yellow),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " [Esc] to cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::Black));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::TuiEvent;

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
        assert!(text.contains("Ctrl+C"), "status should show Ctrl+C: {text}");
        assert!(text.contains("Ctrl+L"), "status should show Ctrl+L: {text}");
        assert!(text.contains("Ctrl+D"), "status should show Ctrl+D: {text}");
        assert!(text.contains("/help"), "status should show /help: {text}");
    }

    #[test]
    fn test_status_bar_generating_shows_interrupt_hint() {
        let mut app = App::new();
        app.state = AppState::Generating;
        let text = status_bar_text(&app);
        assert!(
            text.contains("Generating"),
            "status should show Generating: {text}"
        );
        assert!(
            text.contains("Ctrl+C"),
            "should show interrupt hint: {text}"
        );
    }

    #[test]
    fn test_status_bar_interrupted_shows_message() {
        let mut app = App::new();
        app.state = AppState::Interrupted;
        let text = status_bar_text(&app);
        assert!(
            text.contains("Interrupted"),
            "status should show Interrupted: {text}"
        );
    }

    #[test]
    fn test_status_bar_includes_connection_indicator() {
        let app = App::new();
        let text = status_bar_text(&app);
        assert!(
            text.contains("●")
                || text.contains("○")
                || text.contains("connected")
                || text.contains("disconnected"),
            "status bar should include a connection indicator, got: {text}"
        );
    }

    #[test]
    fn test_tool_card_color_for_read() {
        assert_eq!(tool_card_color("read"), Color::Blue);
    }

    #[test]
    fn test_tool_card_color_for_edit() {
        assert_eq!(tool_card_color("edit"), Color::Magenta);
    }

    #[test]
    fn test_tool_card_color_for_bash() {
        assert_eq!(tool_card_color("bash"), Color::Yellow);
    }

    #[test]
    fn test_tool_card_color_for_search_grep() {
        assert_eq!(tool_card_color("search_grep"), Color::Cyan);
    }

    #[test]
    fn test_tool_card_color_for_write() {
        assert_eq!(tool_card_color("write"), Color::Green);
    }

    #[test]
    fn test_tool_card_color_unknown() {
        assert_eq!(tool_card_color("unknown"), Color::DarkGray);
    }

    // ── VCS tool card color tests ──────────────────────────────────────

    #[test]
    fn test_tool_card_color_for_git_read() {
        assert_eq!(tool_card_color("git_read"), Color::Rgb(100, 180, 100));
    }

    #[test]
    fn test_tool_card_color_for_git_write() {
        assert_eq!(tool_card_color("git_write"), Color::Rgb(255, 140, 0));
    }

    #[test]
    fn test_tool_card_color_for_github() {
        assert_eq!(tool_card_color("github"), Color::Rgb(255, 255, 255));
    }

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
            cell_str.contains("Allow session"),
            "modal should show Allow session option"
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

    // ── Task board view tests ──────────────────────────────────────────

    #[test]
    fn test_task_state_color_todo() {
        assert_eq!(task_state_color("todo"), Color::Gray);
    }

    #[test]
    fn test_task_state_color_in_progress() {
        assert_eq!(task_state_color("in_progress"), Color::Yellow);
    }

    #[test]
    fn test_task_state_color_blocked() {
        assert_eq!(task_state_color("blocked"), Color::Red);
    }

    #[test]
    fn test_task_state_color_in_review() {
        assert_eq!(task_state_color("in_review"), Color::Blue);
    }

    #[test]
    fn test_task_state_color_done() {
        assert_eq!(task_state_color("done"), Color::Green);
    }

    #[test]
    fn test_task_state_color_cancelled() {
        assert_eq!(task_state_color("cancelled"), Color::DarkGray);
    }

    #[test]
    fn test_task_state_color_unknown() {
        assert_eq!(task_state_color("unknown_state"), Color::DarkGray);
    }

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
        assert!(
            cell_str.contains("Conversation"),
            "tab header should show Conversation tab"
        );
        assert!(
            cell_str.contains("Tasks"),
            "tab header should show Tasks tab"
        );
        assert!(
            cell_str.contains("Graph"),
            "tab header should show Graph tab"
        );
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
            cell_str.contains("No tasks"),
            "task board should show empty state message"
        );
    }

    #[test]
    fn test_render_task_board_with_tasks() {
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
        assert!(
            cell_str.contains("blocked by TSK-001"),
            "task board should show blocked_by sub-entry"
        );
    }

    #[test]
    fn test_render_task_graph_placeholder() {
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
            cell_str.contains("coming soon"),
            "graph placeholder should show coming soon"
        );
    }

    #[test]
    fn test_render_task_detail_shows_task_id_in_status() {
        let mut app = App::new();
        app.active_view = crate::app::AppView::TaskDetail("TSK-001".to_string());
        let text = status_bar_text(&app);
        assert!(
            text.contains("TSK-001"),
            "status bar should show task ID in detail view: {text}"
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
            cell_str.contains("in_progress"),
            "detail view should show task state, got: {cell_str}"
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
            cell_str.contains("high"),
            "detail view should show task priority, got: {cell_str}"
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

        // With scroll_offset = 0, all messages should be visible
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf0 = terminal.backend().buffer();
        let text0: String = buf0.content.iter().map(|c| c.symbol()).collect();
        assert!(text0.contains("first message"), "first message should be visible at offset 0");

        // With scroll_offset = 2, "first message" should be skipped
        app.scroll_offset = 2;
        terminal.draw(|f| render(f, &app)).unwrap();
        let buf1 = terminal.backend().buffer();
        let text1: String = buf1.content.iter().map(|c| c.symbol()).collect();
        // "third message" should still be visible
        assert!(text1.contains("third message"), "third message should be visible when scrolled");
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
}
