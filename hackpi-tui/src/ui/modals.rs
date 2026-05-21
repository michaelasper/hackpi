use crate::app::App;
use crate::interaction::app_key_context;
use crate::theme::Theme;
use crate::ui::modal_rect;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Render the slash command autocomplete popover above the input area.
pub(crate) fn render_autocomplete_modal(
    frame: &mut Frame,
    input_area: Rect,
    app: &App,
    theme: &Theme,
) {
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

    // Position above the input area, indented from the left, clamped to top of screen.
    // Clamp x so the modal doesn't overflow the right edge of the terminal.
    let x = (input_area.x + 2).min(frame_area.width.saturating_sub(modal_width));
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
        theme.border_accent.add_modifier(Modifier::BOLD),
    )));

    // Command items — render the visible slice [scroll_offset..scroll_offset+display_count]
    let display_count: usize = filtered.len().min(max_visible);
    let visible_slice = filtered.iter().skip(scroll_offset).take(display_count);
    for (i, cmd) in visible_slice.enumerate() {
        let actual_index = scroll_offset + i;
        let is_selected = actual_index == app.autocomplete_selected;
        let cursor = if is_selected { "▸ " } else { "  " };

        let cmd_style = if is_selected {
            theme.fg_emphasis.bg(Color::DarkGray)
        } else {
            theme.fg_default
        };

        let desc_style = if is_selected {
            theme.fg_muted.bg(Color::DarkGray)
        } else {
            theme.fg_muted
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
        lines.push(Line::from(Span::styled(hint, theme.fg_muted)));
    }

    lines.push(Line::from(Span::styled(
        "  ↑/↓ navigate  Tab select  Enter submit  Esc close",
        theme.fg_muted,
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_accent)
        .style(theme.surface_modal);

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}

#[allow(clippy::vec_init_then_push)]
pub(crate) fn render_permission_modal(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let prompt = match &app.pending_permission {
        Some(p) => p,
        None => return,
    };

    // Responsive modal sizing: 90% width, 85% height — no artificial width cap
    let modal_area = modal_rect(area, area.width, 30, 90, 85);

    // ── Dim the background so the modal is the only visual focus ──────
    frame.buffer_mut().set_style(
        area,
        Style::default()
            .bg(Color::Black)
            .add_modifier(Modifier::DIM),
    );

    // Clear the area behind the modal (restore to default for modal rendering)
    frame.render_widget(Clear, modal_area);

    let reason = &prompt.reason;
    let guard_name = format!("{}", reason.guard);

    // Build modal content lines (NOT truncated; Paragraph::wrap handles overflow)
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " ⚠ Permission Required ",
        theme.status_error.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Detail rows — full text, no silent truncation
    lines.push(Line::from(vec![
        Span::styled(" Pattern: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&reason.details),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Tool:    ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&reason.tool),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Guard:   ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(&guard_name),
    ]));
    lines.push(Line::from(""));

    // ── Group 1: This request (one-off decisions) ──────────────────
    lines.push(Line::from(Span::styled(
        " This request",
        theme.fg_default.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "   [1] Allow once",
        theme.status_warning,
    )));
    lines.push(Line::from(Span::styled(
        "   [3] Deny",
        theme.status_warning,
    )));
    lines.push(Line::from(""));

    // ── Group 2: This session ──────────────────────────────────────
    lines.push(Line::from(Span::styled(" This session", theme.fg_emphasis)));
    lines.push(Line::from(Span::styled(
        "   [2] Allow until exit",
        theme.fg_emphasis,
    )));
    lines.push(Line::from(""));

    // ── Group 3: Persistent rule ───────────────────────────────────
    if prompt.confirming_always_allow {
        // Two-step confirmation state: show confirmation prompt
        lines.push(Line::from(Span::styled(
            " Persistent rule (saved to guardrails config)",
            theme.status_warning.add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "   [4] Press 4 again to confirm persistent allow",
            theme.status_warning.add_modifier(Modifier::BOLD),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " Persistent rule (saved to guardrails config)",
            theme.status_warning.add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "   [4] Always allow this pattern",
            theme.status_warning,
        )));
    }
    lines.push(Line::from(Span::styled(
        "   [5] Always deny this pattern",
        theme.status_warning,
    )));
    lines.push(Line::from(""));

    // Esc hint — explicitly says "Deny" to match the implemented action
    lines.push(Line::from(Span::styled(" [Esc] Deny", theme.fg_muted)));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_danger)
        .style(theme.surface_modal);

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}

/// Render the inline task creation prompt overlaid on the input area.
pub(crate) fn render_task_create_prompt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
            theme.tool_task.add_modifier(Modifier::BOLD),
        )))
        .style(theme.surface_modal),
        input_area[0],
    );

    // Input field with cursor
    let input_text = format!(" {}█", app.task_create_input);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(input_text, theme.fg_default)))
            .style(theme.surface_modal),
        input_area[1],
    );

    // Hint line
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " Enter to create · Esc to cancel",
            theme.fg_muted,
        )))
        .style(theme.surface_modal),
        input_area[2],
    );
}

/// Render the contextual help overlay showing key bindings for the current context.
///
/// Uses a centered modal similar to the permission prompt. Content is generated
/// dynamically from the `KEY_BINDINGS` table, filtered by `app_key_context`.
/// Footer bindings are listed first, followed by additional bindings.
pub(crate) fn render_help_overlay(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let context = app_key_context(app);
    let bindings = crate::interaction::help_bindings(context);

    // Responsive modal: 70% width, capped at 60. Height adapts to content.
    let binding_count = bindings.len() as u16;
    let preferred_height = (binding_count + 4).clamp(8, 24);
    let modal_area = modal_rect(area, 60, preferred_height, 70, 70);

    frame.render_widget(Clear, modal_area);

    let context_name = format!("{context:?}");
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        format!(" ⌨ Help — {context_name} "),
        theme.border_accent.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for binding in &bindings {
        let key_style = if binding.footer {
            theme.fg_emphasis
        } else {
            theme.fg_muted
        };
        let action_style = if binding.footer {
            theme.fg_emphasis
        } else {
            theme.fg_muted
        };

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(format!("{:<12}", binding.key), key_style),
            Span::styled(binding.action, action_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press Esc to close",
        theme.fg_muted,
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_accent)
        .style(theme.surface_modal);

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}
