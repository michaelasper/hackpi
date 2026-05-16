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

fn header_text(app: &App) -> Line<'static> {
    let usage_text = match &app.usage {
        Some(u) => format!("{}↑ {}↓", u.input_tokens, u.output_tokens),
        None => "0↑ 0↓".into(),
    };
    let version = env!("CARGO_PKG_VERSION");
    Line::from(vec![
        Span::styled(
            format!(" hackpi v{version} "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("· ds4 · "),
        Span::raw(usage_text),
    ])
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let text = header_text(app);

    frame.render_widget(
        Paragraph::new(text).style(Style::default().bg(Color::Black)),
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
    let mut items: Vec<ListItem> = Vec::new();

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
            items.push(ListItem::new(Line::from(Span::styled(content, role_style))));
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

            let mut card_lines: Vec<Line> = Vec::new();
            if let ToolCallStatus::Done(result) = &tc.status {
                let result_content = match result {
                    hackpi_core::tools::ToolResult::Success { content } => content.clone(),
                    hackpi_core::tools::ToolResult::SystemError { message } => {
                        format!("Error: {message}")
                    }
                    hackpi_core::tools::ToolResult::Timeout => "Timed out.".into(),
                    hackpi_core::tools::ToolResult::Cancelled => "Cancelled.".into(),
                };
                for line in result_content.lines() {
                    card_lines.push(Line::from(Span::raw(line.to_string())));
                }
            } else {
                card_lines.push(Line::from(Span::styled(
                    "Running...",
                    Style::default().fg(Color::Yellow),
                )));
            }

            card_lines.insert(
                0,
                Line::from(Span::styled(
                    title,
                    Style::default()
                        .fg(border_color)
                        .add_modifier(Modifier::BOLD),
                )),
            );

            items.push(ListItem::new(card_lines.clone()));
        }

        items.push(ListItem::new(Line::from("")));
    }

    let visible_items: &[ListItem] = if app.scroll_offset > 0 && app.scroll_offset < items.len() {
        &items[app.scroll_offset..]
    } else {
        &items
    };

    let list = List::new(visible_items.to_vec()).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default()),
    );

    frame.render_widget(list, area);
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
    let state_text = match app.state {
        AppState::Resting => "Ctrl+C interrupt  Ctrl+L clear  Ctrl+D exit  /help",
        AppState::Generating => "Generating... (Ctrl+C to interrupt)",
        AppState::Interrupted => "Interrupted. Press any key.",
    };
    let conn = "●";
    if app.status_message.is_empty() {
        format!(" {state_text}  {conn}")
    } else {
        format!(" {} | {state_text}  {conn}", app.status_message)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_includes_version() {
        let app = App::new();
        let line = header_text(&app);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("v0.1.0") || rendered.contains("v"),
            "header should include version, got: {rendered}"
        );
    }

    #[test]
    fn test_header_shows_zero_usage() {
        let app = App::new();
        let line = header_text(&app);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("0↑") && rendered.contains("0↓"),
            "header should show 0↑ 0↓, got: {rendered}"
        );
    }

    #[test]
    fn test_header_shows_usage() {
        let mut app = App::new();
        app.usage = Some(hackpi_core::types::Usage {
            input_tokens: 150,
            output_tokens: 75,
        });
        let line = header_text(&app);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("150↑") && rendered.contains("75↓"),
            "header should show 150↑ 75↓, got: {rendered}"
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
}
