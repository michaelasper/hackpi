use crate::app::App;
use crate::interaction::app_key_context;
use crate::theme::Theme;
use crate::ui::{modal_rect, truncate_to_width};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

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

    let frame_area = frame.area();
    let is_color = theme.fg_default.fg.is_some();

    // Modal dimensions — fixed maximum width, capped by terminal width.
    const MAX_VISIBLE: usize = 10;
    let item_count = filtered.len().min(MAX_VISIBLE);
    // Height: top border (1) + title (1) + items + optional "more" (1) + hint (1) + bottom border (1)
    let modal_height = (item_count + 5).min(16) as u16;

    // Position above the input area, clamped so it doesn't cover row 0 (tab header).
    let y = input_area.y.saturating_sub(modal_height).max(1);

    // Modal width: fill available width with some margin, clamped 40..60.
    let modal_width = frame_area.width.clamp(40, 60u16);

    // Clamp x so the modal doesn't overflow the right edge of the terminal.
    let x = (input_area.x + 2).min(frame_area.width.saturating_sub(modal_width));

    let modal_area = Rect {
        x,
        y,
        width: modal_width,
        height: modal_height,
    };

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    // Compute scroll offset so the selected item stays within the visible window.
    let scroll_offset = if app.autocomplete_selected >= MAX_VISIBLE {
        app.autocomplete_selected - MAX_VISIBLE + 1
    } else {
        0
    };

    // Inner width excludes left/right border characters.
    let inner_width = modal_width.saturating_sub(2) as usize;

    // Column allocation from available inner width:
    //   Cursor "▸ " = 2 columns, gap "  " = 2 columns.
    //   Available for name + description = inner_width - 4.
    //   Command column: sized to the widest command name (display-width),
    //   clamped to at most 50% of available (so descriptions get space)
    //   and at least 10 columns.
    //   Description column: whatever remains (truncated as needed).
    let available = inner_width.saturating_sub(4);
    let widest_cmd = filtered
        .iter()
        .map(|cmd| UnicodeWidthStr::width(cmd.name))
        .max()
        .unwrap_or(10);
    let cmd_width = widest_cmd.min(available / 2).max(10);
    let desc_width = available.saturating_sub(cmd_width);

    // Build list lines
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " Slash Commands ",
        theme.border_accent.add_modifier(Modifier::BOLD),
    )));

    // Command items — render the visible slice [scroll_offset..scroll_offset+display_count]
    let display_count: usize = filtered.len().min(MAX_VISIBLE);
    for (i, cmd) in filtered
        .iter()
        .skip(scroll_offset)
        .take(display_count)
        .enumerate()
    {
        let actual_index = scroll_offset + i;
        let is_selected = actual_index == app.autocomplete_selected;

        let cursor = if is_selected { "▸ " } else { "  " };

        // Truncate and pad command name to column width
        let display_name = truncate_to_width(cmd.name, cmd_width);
        let name_width = UnicodeWidthStr::width(display_name.as_str());
        let name_pad = cmd_width.saturating_sub(name_width);
        let name_padded = if name_pad > 0 {
            let mut s = String::with_capacity(display_name.len() + name_pad);
            s.push_str(&display_name);
            for _ in 0..name_pad {
                s.push(' ');
            }
            s
        } else {
            display_name
        };

        // Truncate and pad description to column width
        let display_desc = truncate_to_width(cmd.description, desc_width);
        let desc_width_actual = UnicodeWidthStr::width(display_desc.as_str());
        let desc_pad = desc_width.saturating_sub(desc_width_actual);
        let desc_padded = if desc_pad > 0 {
            let mut s = String::with_capacity(display_desc.len() + desc_pad);
            s.push_str(&display_desc);
            for _ in 0..desc_pad {
                s.push(' ');
            }
            s
        } else {
            display_desc
        };

        if is_selected {
            if is_color {
                // Full-row selection background in color mode
                let cmd_style = theme.fg_emphasis.bg(Color::DarkGray);
                let desc_style = theme.fg_muted.bg(Color::DarkGray);
                lines.push(Line::from(vec![
                    Span::styled(cursor.to_string(), cmd_style),
                    Span::styled(name_padded, cmd_style),
                    Span::styled("  ", cmd_style),
                    Span::styled(desc_padded, desc_style),
                ]));
            } else {
                // Reversed modifier for monochrome / NO_COLOR mode
                let cmd_style = theme.fg_emphasis.add_modifier(Modifier::REVERSED);
                let desc_style = theme.fg_muted.add_modifier(Modifier::REVERSED);
                lines.push(Line::from(vec![
                    Span::styled(cursor.to_string(), cmd_style),
                    Span::styled(name_padded, cmd_style),
                    Span::styled("  ", cmd_style),
                    Span::styled(desc_padded, desc_style),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled(cursor.to_string(), theme.fg_emphasis),
                Span::styled(name_padded, theme.fg_emphasis),
                Span::styled("  ", theme.fg_emphasis),
                Span::styled(desc_padded, theme.fg_muted),
            ]));
        }
    }

    // Hint line — show when total filtered items exceed the visible window
    if filtered.len() > MAX_VISIBLE {
        let remaining = filtered.len().saturating_sub(scroll_offset + MAX_VISIBLE);
        let before = scroll_offset;
        let hint = match (before, remaining) {
            (0, r) => format!("  ↓ {r} more"),
            (b, 0) => format!("  ↑ {b} above"),
            (b, r) => format!("  ↑ {b} above · ↓ {r} more"),
        };
        lines.push(Line::from(Span::styled(
            truncate_to_width(&hint, inner_width),
            theme.fg_muted,
        )));
    }

    // Navigation hint — truncated to inner width
    lines.push(Line::from(Span::styled(
        truncate_to_width(
            "  ↑/↓ navigate  Tab select  Enter submit  Esc close",
            inner_width,
        ),
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
