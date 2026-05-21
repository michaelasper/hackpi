use crate::app::{App, ConversationEntryKind, Severity, ToolCallDisplay, ToolCallStatus};
use crate::theme::{
    tool_card_style, tool_status_label, tool_status_style, tool_status_symbol, Theme,
};
use crate::ui::{count_visual_lines, truncate_for_display};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub(crate) fn user_prefix() -> &'static str {
    " ○ me: "
}

pub(crate) fn assistant_prefix() -> &'static str {
    " ● assistant: "
}

pub(crate) fn render_conversation(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.conversation {
        match &entry.kind {
            ConversationEntryKind::SystemError {
                severity,
                recovery_hint,
            } => {
                let (label, style) = match severity {
                    Severity::Error => (" ERROR ", theme.status_error),
                    Severity::Warning => (" WARNING ", theme.status_warning),
                    Severity::Info => (" INFO ", theme.status_info),
                };
                let error_text = &entry.text;
                let content_width = area.width.saturating_sub(6) as usize;

                // Build a bordered error card
                let top = format!(
                    "┌─{label}─{:─>width$}┐",
                    "",
                    width = content_width
                        .saturating_sub(label.len() + 4)
                        .saturating_sub(3)
                );
                lines.push(Line::from(Span::styled(top, style)));

                for line_content in error_text.lines() {
                    let truncated = truncate_for_display(line_content, content_width);
                    lines.push(Line::from(Span::styled(format!("│ {truncated}"), style)));
                }

                if let Some(hint) = recovery_hint {
                    let hint_truncated = truncate_for_display(hint, content_width);
                    lines.push(Line::from(Span::styled(
                        format!("│ ⤷ {hint_truncated}"),
                        theme.status_info,
                    )));
                }

                let bottom_width = area.width.saturating_sub(2);
                let bottom = format!("└{}┘", "─".repeat(bottom_width as usize));
                lines.push(Line::from(Span::styled(bottom, style)));

                lines.push(Line::from(""));
                continue;
            }
            ConversationEntryKind::Message => {}
        }

        let prefix = match entry.role.as_str() {
            "user" => user_prefix(),
            "assistant" => assistant_prefix(),
            _ => "   ",
        };

        let role_style = match entry.role.as_str() {
            "user" => theme.role_user,
            "assistant" => theme.role_assistant,
            _ => theme.fg_default,
        };

        if !entry.text.is_empty() {
            let content = format!("{prefix}{}", entry.text);
            lines.push(Line::from(Span::styled(content, role_style)));
        }

        for tc in &entry.tool_calls {
            render_tool_card(&mut lines, tc, area.width as usize, theme);
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

// ── Tool card component ────────────────────────────────────────────

/// Render a tool call as a bordered action card with structured summary.
///
/// Card format:
/// ```text
/// ┌─ ✓ read  src/main.rs [Success] ──┐
/// │ file content                      │
/// └───────────────────────────────────┘
/// ```
///
/// The card width adapts to the conversation area width. Status uses
/// semantic colors (green=success, red=error, yellow=running/warning).
pub(crate) fn render_tool_card(
    lines: &mut Vec<Line>,
    tc: &ToolCallDisplay,
    area_width: usize,
    theme: &Theme,
) {
    let card_style = tool_card_style(&tc.name, theme);
    let tool_status_s = tool_status_style(&tc.status, theme);
    let status_symbol = tool_status_symbol(&tc.status);
    let status_label = tool_status_label(&tc.status);

    let title = tc.summary.title();

    // Build the title portion for the top border
    let title_content = format!(" {status_symbol} {title} [{status_label}] ");
    let top_width = area_width.saturating_sub(4); // ┌─ and ─┐
    let filler_needed = top_width.saturating_sub(title_content.len());
    let top_border = if filler_needed > 0 {
        format!("┌─{title_content}{}─┐", "─".repeat(filler_needed))
    } else {
        format!("┌─{title_content}─┐")
    };

    // Top border with title — use tool-type color
    lines.push(Line::from(Span::styled(top_border, card_style)));

    // Content lines (with │ prefix)
    match &tc.status {
        ToolCallStatus::Done(result) => match result {
            hackpi_core::tools::ToolResult::Success { content } => {
                for line_content in content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("│ {line_content}"),
                        theme.fg_default,
                    )));
                }
            }
            hackpi_core::tools::ToolResult::SystemError { message } => {
                for line_content in message.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("│ {line_content}"),
                        tool_status_s,
                    )));
                }
            }
            hackpi_core::tools::ToolResult::Timeout => {
                lines.push(Line::from(Span::styled("│ Timed out.", tool_status_s)));
            }
            hackpi_core::tools::ToolResult::Cancelled => {
                lines.push(Line::from(Span::styled("│ Cancelled.", tool_status_s)));
            }
        },
        ToolCallStatus::Running => {
            lines.push(Line::from(Span::styled("│ Running…", theme.status_running)));
        }
    }

    // Bottom border
    let bottom_width = area_width.saturating_sub(2);
    let bottom_border = format!("└{}┘", "─".repeat(bottom_width));
    lines.push(Line::from(Span::styled(bottom_border, card_style)));
}
