use crate::app::{App, DiagnosticsEntry};
use crate::events::DiagnosticLevel;
use crate::theme::Theme;
use crate::ui::truncate_for_display;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Render the diagnostics log view — a scrollable list of protocol-level
/// diagnostic messages (SSE parse failures, stream warnings, etc.) that
/// are stored separately from the conversation viewport.
pub(crate) fn render_diagnostics(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    if app.diagnostics.is_empty() {
        lines.push(Line::from(Span::styled(
            " No diagnostics recorded.",
            theme.fg_muted,
        )));
    } else {
        for entry in &app.diagnostics {
            render_diagnostics_entry(&mut lines, entry, area.width as usize, theme);
        }
    }

    // Summary line at the bottom
    lines.push(Line::from(""));
    let count = app.diagnostics.len();
    let summary = if count == 1 {
        " 1 diagnostic recorded".to_string()
    } else {
        format!(" {count} diagnostics recorded")
    };
    lines.push(Line::from(Span::styled(summary, theme.fg_muted)));

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Render a single diagnostics entry as bordered card with level-based coloring.
fn render_diagnostics_entry(
    lines: &mut Vec<Line>,
    entry: &DiagnosticsEntry,
    area_width: usize,
    theme: &Theme,
) {
    let (label, style) = match entry.level {
        DiagnosticLevel::Error => (" DIAG ", theme.status_error),
        DiagnosticLevel::Warning => (" DIAG ", theme.status_warning),
        DiagnosticLevel::Info => (" DIAG ", theme.status_info),
    };

    let content_width = area_width.saturating_sub(6);

    // Build a bordered diagnostic card
    let level_str = format!("{}", entry.level);
    let top = format!(
        "┌─{label}{level_str}─{:─>width$}┐",
        "",
        width = content_width
            .saturating_sub(label.len() + level_str.len() + 4)
            .saturating_sub(3)
    );
    lines.push(Line::from(Span::styled(top, style)));

    // Timestamp line
    lines.push(Line::from(Span::styled(
        format!(" │ {} ", entry.timestamp),
        theme.fg_muted,
    )));

    // Message content
    for line_content in entry.message.lines() {
        let truncated = truncate_for_display(line_content, content_width);
        lines.push(Line::from(Span::styled(format!(" │ {truncated}"), style)));
    }

    let bottom_width = area_width.saturating_sub(2);
    let bottom = format!(" └{}┘", "─".repeat(bottom_width));
    lines.push(Line::from(Span::styled(bottom, style)));

    lines.push(Line::from(""));
}
