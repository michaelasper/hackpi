use crate::app::{App, AppState, ToolCallStatus};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, chunks[0], app);
    render_conversation(frame, chunks[1], app);
    render_input(frame, chunks[2], app);
    render_status(frame, chunks[3], app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let usage_text = match &app.usage {
        Some(u) => format!("{}↑ {}↓", u.input_tokens, u.output_tokens),
        None => "0↑ 0↓".into(),
    };

    let text = Line::from(vec![
        Span::styled(" hackpi ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("· ds4 · "),
        Span::raw(&usage_text),
    ]);

    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn render_conversation(frame: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    for entry in &app.conversation {
        let prefix = match entry.role.as_str() {
            "user" => " ○ ",
            "assistant" => " ● ",
            _ => "   ",
        };

        let role_style = match entry.role.as_str() {
            "user" => Style::default().fg(Color::Green),
            "assistant" => Style::default().fg(Color::Cyan),
            _ => Style::default(),
        };

        if !entry.text.is_empty() {
            let content = format!("{prefix}{}", entry.text);
            items.push(ListItem::new(Line::from(Span::styled(content, role_style))));
        }

        for tc in &entry.tool_calls {
            let (status_symbol, status_color) = match &tc.status {
                ToolCallStatus::Running => ("⋯", Color::Yellow),
                ToolCallStatus::Done(result) => match result {
                    hackpi_core::tools::ToolResult::Success { .. } => ("✓", Color::Green),
                    hackpi_core::tools::ToolResult::SystemError { .. } => ("✗", Color::Red),
                    hackpi_core::tools::ToolResult::Timeout => ("⚠", Color::Yellow),
                    hackpi_core::tools::ToolResult::Cancelled => ("⊘", Color::Gray),
                },
            };

            let tool_text = format!("  {status_symbol} {name}", name = tc.name);
            let style = Style::default().fg(status_color);

            items.push(ListItem::new(Line::from(Span::styled(tool_text, style))));
        }

        items.push(ListItem::new(Line::from("")));
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default()),
    );

    frame.render_widget(list, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &App) {
    let input_block = Block::default().borders(Borders::TOP).style(
        Style::default()
            .fg(if matches!(app.state, AppState::Generating) {
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

fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let text = match app.state {
        AppState::Resting => " Ctrl+C interrupt  Ctrl+L clear  Ctrl+D exit  /help",
        AppState::Generating => " Generating... (Ctrl+C to interrupt)",
        AppState::Interrupted => " Interrupted. Press any key.",
    };

    let style = match app.state {
        AppState::Resting => Style::default().fg(Color::DarkGray),
        AppState::Generating => Style::default().fg(Color::Yellow),
        AppState::Interrupted => Style::default().fg(Color::Red),
    };

    let status_text = if !app.status_message.is_empty() {
        format!(" {} | {text}", app.status_message)
    } else {
        format!(" {text}")
    };

    frame.render_widget(
        Paragraph::new(status_text)
            .style(style)
            .wrap(Wrap { trim: true }),
        area,
    );
}
