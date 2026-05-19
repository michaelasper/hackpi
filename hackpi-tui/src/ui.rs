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

    if app.creating_task {
        render_task_create_prompt(frame, chunks[2], app);
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
        "task" => Color::Rgb(255, 200, 0),       // amber — task management
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

    let text = Text::from(lines);
    let area_width = area.width as usize;
    let visible_height = area.height as usize;

    // Calculate total visual height accounting for word wrapping
    let total_height = count_visual_lines(&text, area_width);

    let scroll_y = if app.auto_scroll {
        // Auto-scroll: pin to the bottom of the conversation
        total_height
            .saturating_sub(visible_height)
            .min(u16::MAX as usize) as u16
    } else {
        // Manual scroll: use stored offset, clamped to valid range
        let max_scroll = total_height.saturating_sub(visible_height);
        app.scroll_offset.min(max_scroll).min(u16::MAX as usize) as u16
    };

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default()),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));

    frame.render_widget(paragraph, area);
}

/// Count the total number of visual rows a `Text` will occupy when rendered
/// in an area of the given width, accounting for word wrapping.
fn count_visual_lines(text: &Text, area_width: usize) -> usize {
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
            "  No tasks. Press 'n' to create one.",
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
        "  ↑/↓ navigate  Enter detail  Esc back  n new task  /task move <id> <state>",
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

    // Show the terminal cursor at the current typing position.
    // Only when the user can type (Resting state, no active modals).
    if matches!(app.state, AppState::Resting)
        && app.pending_permission.is_none()
        && !app.creating_task
    {
        let prefix_len: u16 = prefix.len() as u16;
        let cursor_col = input_area.x + prefix_len + app.input_cursor as u16;
        let cursor_row = input_area.y;
        // Clamp cursor to the input area bounds to avoid panics on narrow terminals.
        let clamped_col = cursor_col.min(input_area.right().saturating_sub(1));
        frame.set_cursor_position((clamped_col, cursor_row));
    }
}

/// Spinner frames for the animated loading indicator.
/// Cycles through these while waiting for LLM response.
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

fn status_bar_text(app: &App) -> String {
    let view_hint = match &app.active_view {
        AppView::Conversation => "",
        AppView::TaskBoard => "Tab:Tasks  ",
        AppView::TaskDetail(id) => return format!(" Task: {id}  |  Esc back  ↑/↓ navigate  ●"),
        AppView::TaskGraph => "Tab:Graph  ",
    };
    let state_text: String = match app.state {
        AppState::Resting => "Ctrl+C interrupt  Ctrl+L clear  Ctrl+D exit  /help".into(),
        AppState::Generating => {
            let frame = SPINNER_FRAMES[app.loading_frame % SPINNER_FRAMES.len()];
            format!("Generating... {frame} (Ctrl+C to interrupt)")
        }
        AppState::Interrupted => "Interrupted. Press any key.".into(),
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

    // Compute the widest command name and description to size columns dynamically.
    let max_name_width = filtered
        .iter()
        .map(|cmd| cmd.name.chars().count())
        .max()
        .unwrap_or(10)
        .max(10);
    let max_desc_width = filtered
        .iter()
        .map(|cmd| cmd.description.chars().count())
        .max()
        .unwrap_or(20);

    // Modal dimensions — wide enough for cursor + name column + gap + description + borders.
    // 2 (cursor "▸ ") + name + 2 (gap "  ") + description + 2 (left/right border padding)
    let frame_area = frame.area();
    let min_modal_width = (2 + max_name_width + 2 + max_desc_width + 2).max(50) as u16;
    let modal_width = frame_area.width.min(min_modal_width);
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

    // Compute scroll offset so the selected item stays within the visible window.
    let scroll_offset = if app.autocomplete_selected >= max_visible {
        app.autocomplete_selected - max_visible + 1
    } else {
        0
    };

    // Build list lines
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " Slash Commands ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));

    // Command items — render the visible slice [scroll_offset..scroll_offset+display_count]
    let display_count: usize = filtered.len().min(max_visible);
    let visible_slice = filtered.iter().skip(scroll_offset).take(display_count);
    for (i, cmd) in visible_slice.enumerate() {
        let actual_index = scroll_offset + i;
        let is_selected = actual_index == app.autocomplete_selected;
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

        // Truncate the name to the computed column width if it somehow exceeds it,
        // then pad to ensure consistent column alignment.
        let display_name = if cmd.name.chars().count() > max_name_width {
            let truncated: String = cmd.name.chars().take(max_name_width - 1).collect();
            format!("{truncated}…")
        } else {
            format!("{:<width$}", cmd.name, width = max_name_width)
        };

        lines.push(Line::from(vec![
            Span::styled(cursor.to_string(), cmd_style),
            Span::styled(display_name, cmd_style),
            Span::styled("  ".to_string(), cmd_style), // separator gap
            Span::styled(cmd.description, desc_style),
        ]));
    }

    // Hint line — show when total filtered items exceed the visible window
    if filtered.len() > max_visible {
        let remaining = filtered.len().saturating_sub(scroll_offset + max_visible);
        let before = scroll_offset;
        let hint = match (before, remaining) {
            (0, r) => format!("  ↓ {r} more"),
            (b, 0) => format!("  ↑ {b} above"),
            (b, r) => format!("  ↑ {b} above · ↓ {r} more"),
        };
        lines.push(Line::from(Span::styled(
            hint,
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

/// Render the inline task creation prompt overlaid on the input area.
fn render_task_create_prompt(frame: &mut Frame, area: Rect, app: &App) {
    // Clear the input area
    frame.render_widget(Clear, area);

    let input_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // input field
            Constraint::Length(1), // hint
        ])
        .split(area);

    // Label line
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " New Task:",
            Style::default()
                .fg(Color::Rgb(255, 200, 0))
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(Color::Black)),
        input_area[0],
    );

    // Input field with cursor
    let input_text = format!(" {}█", app.task_create_input);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            input_text,
            Style::default().fg(Color::White),
        )))
        .style(Style::default().bg(Color::Black)),
        input_area[1],
    );

    // Hint line
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Enter to create · Esc to cancel",
            Style::default().fg(Color::DarkGray),
        )))
        .style(Style::default().bg(Color::Black)),
        input_area[2],
    );
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
    fn test_tool_card_color_for_task() {
        assert_eq!(tool_card_color("task"), Color::Rgb(255, 200, 0));
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
        // Use a small terminal to force scrolling
        let backend = TestBackend::new(80, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();

        // Add enough messages to overflow the small area
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
        let backend = TestBackend::new(80, 10);
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
    fn test_render_task_board_empty_state_mentions_n_key() {
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
            cell_str.contains("Press 'n'"),
            "empty state should mention 'n' key, got: {cell_str}"
        );
    }

    #[test]
    fn test_render_task_board_footer_shows_n_key() {
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

        terminal.draw(|f| render(f, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let cell_str: String = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<&str>>()
            .concat();
        assert!(
            cell_str.contains("n new task"),
            "footer should mention 'n new task', got: {cell_str}"
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
}
