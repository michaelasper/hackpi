use crate::app::{
    App, AppView, ConversationEntryKind, Severity, ToolCallDisplay, ToolCallStatus, UiStatus,
};
use crate::interaction::{app_key_context, footer_bindings};
use crate::theme::{
    current_theme, format_task_state, priority_label, priority_style, task_state_style,
    tool_card_style, tool_status_label, tool_status_style, tool_status_symbol, Theme,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

// ── Minimum terminal size gate ──────────────────────────────────────────────

/// Minimum terminal width required for the TUI to render properly.
pub const MIN_TERMINAL_WIDTH: u16 = 80;
/// Minimum terminal height required for the TUI to render properly.
pub const MIN_TERMINAL_HEIGHT: u16 = 24;

/// Returns `true` if the terminal is too small to render the full TUI layout.
///
/// When too small, callers should render [`render_too_small`] instead of the
/// normal layout to avoid clipping, overlapping, or panics.
pub fn is_too_small(area: Rect) -> bool {
    area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT
}

/// Renders a centered "terminal too small" message.
///
/// Displays the minimum required dimensions and the current terminal size so
/// the user knows how much they need to resize.
pub fn render_too_small(frame: &mut Frame, area: Rect) {
    let theme = current_theme();
    let text = format!(
        "Terminal too small\n\
         Minimum: {MIN_TERMINAL_WIDTH}x{MIN_TERMINAL_HEIGHT}\n\
         Current: {}x{}",
        area.width, area.height
    );
    let paragraph = Paragraph::new(text)
        .style(theme.status_error)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

// ── Root layout helper ─────────────────────────────────────────────────────

/// The four major screen regions produced by [`split_root`].
pub struct RootLayout {
    /// Tab header row (height 1).
    pub header: Rect,
    /// Main content area (fills remaining vertical space).
    pub main: Rect,
    /// Input area (height 3).
    pub input: Rect,
    /// Status bar row (height 1).
    pub status: Rect,
}

/// Split the full terminal area into the four standard TUI regions.
///
/// Layout (top to bottom):
/// 1. Tab header (1 line)
/// 2. Main content (fills remaining)
/// 3. Input area (3 lines)
/// 4. Status bar (1 line)
pub fn split_root(area: Rect) -> RootLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab header
            Constraint::Min(1),    // main content
            Constraint::Length(3), // input
            Constraint::Length(1), // status bar
        ])
        .split(area);

    RootLayout {
        header: chunks[0],
        main: chunks[1],
        input: chunks[2],
        status: chunks[3],
    }
}

// ── Responsive modal helpers ───────────────────────────────────────────────

/// Return a rect of the given percentage of `area`, centered within it.
///
/// Both `width_pct` and `height_pct` are clamped to `[1, 100]`. The returned
/// rect is guaranteed to fit within `area`.
pub fn centered_rect(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let w_pct = width_pct.clamp(1, 100);
    let h_pct = height_pct.clamp(1, 100);

    let width = (area.width * w_pct / 100).max(1);
    let height = (area.height * h_pct / 100).max(1);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Return a responsive modal rect that is a percentage of `area`, capped at
/// `preferred_{width,height}`, and never exceeds `area`'s dimensions.
///
/// Use this for modals that should scale with the terminal but not exceed a
/// comfortable maximum size.
pub fn modal_rect(
    area: Rect,
    preferred_width: u16,
    preferred_height: u16,
    width_pct: u16,
    height_pct: u16,
) -> Rect {
    let max_w = (area.width * width_pct / 100).max(1);
    let max_h = (area.height * height_pct / 100).max(1);

    let width = preferred_width.min(max_w).min(area.width);
    let height = preferred_height.min(max_h).min(area.height);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect {
        x,
        y,
        width,
        height,
    }
}

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

fn user_prefix() -> &'static str {
    " ○ me: "
}

fn assistant_prefix() -> &'static str {
    " ● assistant: "
}

fn render_conversation(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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

/// Render the task board list view, grouped by state with counts.
fn render_task_board(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut items: Vec<ListItem> = Vec::new();

    if app.task_list_cache.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  No tasks yet. Press 'n' to create one.",
            theme.fg_muted,
        ))));
        let list = List::new(items).block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default()),
        );
        frame.render_widget(list, area);
        return;
    }

    // Build state groups preserving the original task order.
    // Each group is (state_key, [(task_index, &Task)]).
    let mut groups: Vec<(String, Vec<(usize, &hackpi_tasks::Task)>)> = Vec::new();
    for (i, task) in app.task_list_cache.iter().enumerate() {
        let state_key = &task.state;
        if let Some(last) = groups.last_mut() {
            if last.0 == *state_key {
                last.1.push((i, task));
                continue;
            }
        }
        groups.push((state_key.clone(), vec![(i, task)]));
    }

    // Compute available width for header filler
    let area_width = area.width.saturating_sub(2) as usize;

    // Render each group
    for (group_idx, (state_key, group_tasks)) in groups.iter().enumerate() {
        let state_label = format_task_state(state_key);
        let count = group_tasks.len();
        let header_core = format!("── {state_label} ({count}) ");
        let filler_len = area_width.saturating_sub(header_core.len());
        let header_text = if filler_len > 0 {
            format!("{header_core}{}──", "─".repeat(filler_len))
        } else {
            header_core
        };

        // Section header
        let header_style = task_state_style(state_key, theme);
        items.push(ListItem::new(Line::from(Span::styled(
            header_text,
            header_style.add_modifier(Modifier::BOLD),
        ))));

        for (task_idx, task) in group_tasks {
            let is_selected = *task_idx == app.selected_task_idx;
            let state_style = task_state_style(&task.state, theme);
            let cursor = if is_selected { "▸ " } else { "  " };

            // Main task line
            let mut line_spans: Vec<Span> = Vec::new();

            // Selection cursor
            line_spans.push(Span::styled(
                cursor.to_string(),
                if is_selected {
                    theme.fg_emphasis
                } else {
                    theme.fg_muted
                },
            ));

            // Task ID
            line_spans.push(Span::styled(format!("{} ", task.id), theme.fg_emphasis));

            // State badge (human-readable)
            let state_label = format_task_state(&task.state);
            line_spans.push(Span::styled(format!("[{state_label}] "), state_style));

            // Title
            line_spans.push(Span::styled(
                task.title.clone(),
                if is_selected {
                    theme.fg_emphasis
                } else {
                    theme.fg_default
                },
            ));

            items.push(ListItem::new(Line::from(line_spans)));

            // Show blocked_by as indented sub-entries
            if !task.blocked_by.is_empty() {
                for blocker_id in &task.blocked_by {
                    items.push(ListItem::new(Line::from(Span::styled(
                        format!("      ⬑ blocked by {}", blocker_id),
                        theme.status_error,
                    ))));
                }
            }
        }

        // Blank line between groups (except last)
        if group_idx < groups.len() - 1 {
            items.push(ListItem::new(Line::from("")));
        }
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default()),
    );

    frame.render_widget(list, area);
}

/// Render the task dependency graph view, showing a selected task's
/// blockers and dependents in a tree layout.
///
/// If a task is selected in the task board, its dependency information is
/// shown. Otherwise a helpful message is displayed.
fn render_task_graph(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " Task Dependencies",
        theme.fg_emphasis.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let selected_task = app.task_list_cache.get(app.selected_task_idx);

    if let Some(selected) = selected_task {
        // Show the selected task as the focal point
        let state_label = format_task_state(&selected.state);
        lines.push(Line::from(vec![
            Span::styled(" Selected: ", theme.fg_muted),
            Span::styled(format!("{} ", selected.id), theme.fg_emphasis),
            Span::styled(
                format!("[{}] ", state_label),
                task_state_style(&selected.state, theme),
            ),
            Span::styled(selected.title.clone(), theme.fg_default),
        ]));
        lines.push(Line::from(""));

        // Blockers (tasks this task depends on)
        if !selected.blocked_by.is_empty() {
            lines.push(Line::from(Span::styled(
                " Blocked by:",
                theme.status_warning.add_modifier(Modifier::BOLD),
            )));
            for blocker_id in &selected.blocked_by {
                // Look up blocker details from cache
                let blocker_info = app.task_list_cache.iter().find(|t| t.id == *blocker_id);
                match blocker_info {
                    Some(bt) => {
                        let bt_label = format_task_state(&bt.state);
                        lines.push(Line::from(vec![
                            Span::styled("   ⬑ ", theme.status_error),
                            Span::styled(format!("{} ", bt.id), theme.fg_emphasis),
                            Span::styled(
                                format!("[{}] ", bt_label),
                                task_state_style(&bt.state, theme),
                            ),
                            Span::styled(bt.title.clone(), theme.fg_default),
                        ]));
                    }
                    None => {
                        lines.push(Line::from(Span::styled(
                            format!("   ⬑ {} (not in current view)", blocker_id),
                            theme.status_error,
                        )));
                    }
                }
            }
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(" No blockers.", theme.fg_muted)));
            lines.push(Line::from(""));
        }

        // Blocking (tasks that depend on this task)
        let blocking_ids: Vec<String> = app
            .task_list_cache
            .iter()
            .filter(|t| t.blocked_by.contains(&selected.id))
            .map(|t| t.id.clone())
            .collect();

        if !blocking_ids.is_empty() {
            lines.push(Line::from(Span::styled(
                " Blocks:",
                theme.status_warning.add_modifier(Modifier::BOLD),
            )));
            for blocking_id in &blocking_ids {
                let bt = app.task_list_cache.iter().find(|t| t.id == *blocking_id);
                match bt {
                    Some(bt) => {
                        let bt_label = format_task_state(&bt.state);
                        lines.push(Line::from(vec![
                            Span::styled("   ⤳ ", theme.status_info),
                            Span::styled(format!("{} ", bt.id), theme.fg_emphasis),
                            Span::styled(
                                format!("[{}] ", bt_label),
                                task_state_style(&bt.state, theme),
                            ),
                            Span::styled(bt.title.clone(), theme.fg_default),
                        ]));
                    }
                    None => {
                        lines.push(Line::from(Span::styled(
                            format!("   ⤳ {} (not in current view)", blocking_id),
                            theme.status_info,
                        )));
                    }
                }
            }
        } else {
            lines.push(Line::from(Span::styled(" No dependents.", theme.fg_muted)));
        }
    } else if app.task_list_cache.is_empty() {
        lines.push(Line::from(Span::styled(
            " No tasks loaded. Create or refresh tasks to view dependencies.",
            theme.fg_muted,
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " Select a task from the task board to view its dependencies.",
            theme.fg_muted,
        )));
    }

    lines.push(Line::from(""));

    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default());

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Format a DateTime<Utc> as a local time string (YYYY-MM-DD HH:MM).
fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*dt);
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Render the task detail view showing full task information.
fn render_task_detail(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let task = match &app.task_detail_cache {
        Some(t) => t,
        None => {
            let text = Paragraph::new("Task not found")
                .style(theme.status_error)
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
        theme.fg_emphasis,
    )));
    lines.push(Line::from(""));

    // Title field
    lines.push(Line::from(vec![
        Span::styled("  Title:       ", theme.fg_muted),
        Span::styled(task.title.clone(), theme.fg_emphasis),
    ]));

    // State field (colored, human-readable)
    let state_label = format_task_state(&task.state);
    let state_style = task_state_style(&task.state, theme);
    lines.push(Line::from(vec![
        Span::styled("  State:       ", theme.fg_muted),
        Span::styled(state_label, state_style),
    ]));

    // Priority field (colored, human-readable)
    let priority_str = priority_label(&task.priority);
    let priority_sty = priority_style(&task.priority, theme);
    lines.push(Line::from(vec![
        Span::styled("  Priority:    ", theme.fg_muted),
        Span::styled(priority_str, priority_sty),
    ]));

    // Workflow field
    lines.push(Line::from(vec![
        Span::styled("  Workflow:    ", theme.fg_muted),
        Span::styled(task.workflow.clone(), theme.fg_default),
    ]));

    // Created field
    lines.push(Line::from(vec![
        Span::styled("  Created:     ", theme.fg_muted),
        Span::styled(format_timestamp(&task.created_at), theme.fg_default),
    ]));

    // Updated field
    lines.push(Line::from(vec![
        Span::styled("  Updated:     ", theme.fg_muted),
        Span::styled(format_timestamp(&task.updated_at), theme.fg_default),
    ]));

    // Assignee field
    let assignee_display = task.assignee.as_deref().unwrap_or(em_dash);
    lines.push(Line::from(vec![
        Span::styled("  Assignee:    ", theme.fg_muted),
        Span::styled(assignee_display.to_string(), theme.fg_default),
    ]));

    // Labels field
    let labels_display = if task.labels.is_empty() {
        em_dash.to_string()
    } else {
        task.labels.join(", ")
    };
    lines.push(Line::from(vec![
        Span::styled("  Labels:      ", theme.fg_muted),
        Span::styled(labels_display, theme.fg_default),
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
        Span::styled("  Blocked by:  ", theme.fg_muted),
        Span::styled(
            blocked_by_display,
            if app.task_detail_blocked_by.is_empty() {
                theme.fg_muted
            } else {
                theme.status_error
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
        Span::styled("  Blocking:    ", theme.fg_muted),
        Span::styled(
            blocking_display,
            if app.task_detail_blocking.is_empty() {
                theme.fg_muted
            } else {
                theme.status_warning
            },
        ),
    ]));

    lines.push(Line::from(""));

    // Description section
    lines.push(Line::from(Span::styled("  Description:", theme.fg_muted)));
    if task.description.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {em_dash}"),
            theme.fg_muted,
        )));
    } else {
        for desc_line in task.description.lines() {
            lines.push(Line::from(Span::raw(format!("  {desc_line}"))));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  /task move {id} done  or  /task block <id> {id}"),
        theme.fg_muted,
    )));

    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default());

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

// ── Unicode-aware text measurement helpers ────────────────────────────

/// Calculate the terminal display width of the text before the given
/// character cursor position. Uses `unicode-width` to correctly handle CJK,
/// emoji, combining marks, and other wide or zero-width characters.
pub fn display_width_prefix(s: &str, char_cursor: usize) -> usize {
    let byte_pos = s
        .char_indices()
        .nth(char_cursor)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    UnicodeWidthStr::width(&s[..byte_pos])
}

/// Calculate the (row, col) position of the cursor within the input area,
/// accounting for:
///
/// * The prompt prefix width (e.g. `"> "` is 2 columns)
/// * Explicit newlines (`\n`) inserted via Shift+Enter
/// * Terminal display widths of individual characters (CJK = 2, emoji = 2, etc.)
/// * Word-wrapping within the input area when a logical line exceeds the width
///
/// Returns `(row_offset, col_offset)` **relative to the input area origin**.
/// The caller adds `input_area.y` and `input_area.x` respectively.
pub fn cursor_position_for_input(
    input: &str,
    char_cursor: usize,
    input_area_width: u16,
    prefix_width: u16,
) -> (u16, u16) {
    let area_w = input_area_width as usize;
    if area_w == 0 {
        return (0, prefix_width);
    }

    // Convert char-indexed cursor to byte offset
    let byte_pos = input
        .char_indices()
        .nth(char_cursor)
        .map(|(i, _)| i)
        .unwrap_or(input.len());

    let before_cursor = &input[..byte_pos];

    // Cursor at very start — position right after the prefix
    if before_cursor.is_empty() {
        return (0, prefix_width);
    }

    // Split the pre-cursor text by explicit newlines to handle multiline input.
    // Each segment is a "logical line". The first logical line has the prompt
    // prefix prepended; subsequent lines do not.
    let parts: Vec<&str> = before_cursor.split('\n').collect();
    let line_count = parts.len();

    let mut row: u16 = 0;
    let mut col: u16 = prefix_width;

    for (i, part) in parts.iter().enumerate() {
        let part_width = UnicodeWidthStr::width(*part);

        let effective_width = if i == 0 {
            // First logical line includes the prompt prefix
            prefix_width as usize + part_width
        } else {
            part_width
        };

        if i == line_count - 1 {
            // Last segment — cursor is somewhere within this visual line.
            // Integer division gives the wrapped row offset within this segment;
            // remainder gives the column.
            row += (effective_width / area_w) as u16;
            col = (effective_width % area_w) as u16;
        } else {
            // Complete logical line before the cursor.
            // Count how many visual rows it occupies (even empty lines = 1 row).
            let visual_rows = if effective_width == 0 {
                1
            } else {
                effective_width.div_ceil(area_w)
            };
            row += visual_rows as u16;
        }
    }

    (row, col)
}

/// Truncate a string to fit within a maximum terminal display width.
///
/// Uses `unicode-width` to ensure CJK, emoji, and combining characters are
/// measured by their rendered column width, not their byte or scalar count.
/// Appends "…" when truncation occurs.
pub fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let mut current_width: usize = 0;
    let mut result = String::new();
    let mut truncated = false;

    for c in s.chars() {
        let c_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + c_width > max_width {
            truncated = true;
            break;
        }
        result.push(c);
        current_width += c_width;
    }

    if truncated {
        // If we can fit the ellipsis character (width 1), append it
        if current_width < max_width {
            result.push('…');
        }
    }

    result
}

fn render_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let input_block = Block::default()
        .borders(Borders::TOP)
        .style(if app.ui_status.is_active() {
            theme.input_muted
        } else {
            theme.input_active
        });

    let input_area = input_block.inner(area);
    frame.render_widget(input_block, area);

    let prefix = "> ";
    let display = if app.input.is_empty() && !app.ui_status.is_active() {
        format!("{prefix}Type a message…")
    } else {
        format!("{prefix}{}", app.input)
    };

    let paragraph = Paragraph::new(display).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, input_area);

    // Show the terminal cursor at the current typing position.
    // Only when the focus target is ConversationInput.
    if matches!(
        app.focus_target(),
        crate::interaction::FocusTarget::ConversationInput
    ) && !app.help_visible
    {
        let prefix_width: u16 = UnicodeWidthStr::width(prefix) as u16;
        let (cursor_row_offset, cursor_col_offset) =
            cursor_position_for_input(&app.input, app.input_cursor, input_area.width, prefix_width);
        let cursor_col = input_area.x + cursor_col_offset;
        let cursor_row = input_area.y + cursor_row_offset;
        // Clamp cursor to the input area bounds to avoid panics on narrow terminals.
        let clamped_col = cursor_col.min(input_area.right().saturating_sub(1));
        frame.set_cursor_position((clamped_col, cursor_row));
    }
}

/// Spinner frames for the animated loading indicator.
/// Cycles through these while waiting for LLM response.
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Build the status text for the UiStatus indicator portion of the status bar.
fn ui_status_label(status: &UiStatus, loading_frame: usize) -> String {
    match status {
        UiStatus::Idle => String::new(),
        UiStatus::Generating => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Generating… {frame}")
        }
        UiStatus::RunningTool { name } => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Running {name}… {frame}")
        }
        UiStatus::LoadingTasks => {
            let frame = SPINNER_FRAMES[loading_frame % SPINNER_FRAMES.len()];
            format!("Loading tasks... {frame}")
        }
        UiStatus::WaitingForPermission => "Waiting for permission…".into(),
        UiStatus::Error { message, severity } => {
            let tag = match severity {
                Severity::Info => "INFO",
                Severity::Warning => "WARN",
                Severity::Error => "ERR",
            };
            // Truncate long error messages for the status bar (char-safe)
            let display = truncate_for_display(message, 50);
            format!("[{tag}] {display}")
        }
    }
}

fn status_bar_text(app: &App) -> String {
    let context = app_key_context(app);
    let is_detail = matches!(app.active_view, AppView::TaskDetail(_));

    // Task detail shows a bespoke status line with task ID
    if is_detail {
        if let Some(task) = &app.task_detail_cache {
            return format!(" Task: {}  {}", task.id, app.connection_health.label());
        }
    }

    // State indicator
    let state_text = ui_status_label(&app.ui_status, app.loading_frame);

    // Dynamic footer hints from KEY_BINDINGS table
    let bindings = footer_bindings(context);
    let binding_text: String = bindings
        .iter()
        .map(|b| format!("[{}] {}", b.key, b.action))
        .collect::<Vec<_>>()
        .join("  ");

    // Extra info message if present
    let info_prefix = match &app.info_message {
        Some(msg) => format!("{msg} | "),
        None => String::new(),
    };

    let health_label = app.connection_health.label();

    if !state_text.is_empty() {
        if !binding_text.is_empty() {
            format!(" {info_prefix}{state_text}  ·  {binding_text}  {health_label}")
        } else {
            format!(" {info_prefix}{state_text}  {health_label}")
        }
    } else if !binding_text.is_empty() {
        format!(" {info_prefix}{binding_text}  {health_label}")
    } else if !info_prefix.is_empty() {
        format!(" {info_prefix}{health_label}")
    } else {
        format!(" {health_label}")
    }
}

fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let text = status_bar_text(app);

    // Derive style from UiStatus
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

    // Use Line::raw (no wrapping) so the status bar never wraps to the next
    // line, which would overlap with other layout regions on narrow terminals.
    frame.render_widget(Paragraph::new(Line::from(text)).style(style), area);
}

/// Render the slash command autocomplete popover above the input area.
fn render_autocomplete_modal(frame: &mut Frame, input_area: Rect, app: &App, theme: &Theme) {
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

/// Truncate a string to at most `max_len` characters, appending "…" if truncated.
fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

#[allow(clippy::vec_init_then_push)]
fn render_permission_modal(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let prompt = match &app.pending_permission {
        Some(p) => p,
        None => return,
    };

    // Responsive modal sizing: 85% of terminal width/height, capped at 60x20
    let modal_area = modal_rect(area, 60, 20, 85, 85);

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    let reason = &prompt.reason;
    let guard_name = format!("{}", reason.guard);

    // Determine usable width inside borders (2 chars for left/right borders)
    let content_width = modal_area.width.saturating_sub(2) as usize;

    // Truncate long values to avoid overflowing the modal
    let details_truncated = if content_width > 10 {
        truncate_for_display(&reason.details, content_width.saturating_sub(10))
    } else {
        truncate_for_display(&reason.details, 20)
    };
    let tool_truncated = if content_width > 8 {
        truncate_for_display(&reason.tool, content_width.saturating_sub(8))
    } else {
        truncate_for_display(&reason.tool, 20)
    };
    let guard_truncated = if content_width > 8 {
        truncate_for_display(&guard_name, content_width.saturating_sub(8))
    } else {
        truncate_for_display(&guard_name, 20)
    };

    // Build modal content lines
    let mut lines: Vec<Line> = Vec::new();

    // Title
    lines.push(Line::from(Span::styled(
        " ⚠ Permission Required ",
        theme.status_error.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Details (truncated to modal width)
    lines.push(Line::from(vec![
        Span::styled(" Pattern: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(details_truncated),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Tool:    ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(tool_truncated),
    ]));
    lines.push(Line::from(vec![
        Span::styled(" Guard:   ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(guard_truncated),
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

    // Esc hint
    lines.push(Line::from(Span::styled(" [Esc] Cancel", theme.fg_muted)));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_danger)
        .style(theme.surface_modal);

    let paragraph = Paragraph::new(Text::from(lines))
        .block(block)
        .alignment(Alignment::Left);

    frame.render_widget(paragraph, modal_area);
}

/// Render the inline task creation prompt overlaid on the input area.
fn render_task_create_prompt(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
fn render_help_overlay(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let context = app_key_context(app);
    let bindings = super::interaction::help_bindings(context);

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
fn render_tool_card(lines: &mut Vec<Line>, tc: &ToolCallDisplay, area_width: usize, theme: &Theme) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ConnectionHealth;
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
            cell_str.contains("[Esc] Cancel"),
            "modal should show [Esc] Cancel"
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
    fn test_render_permission_modal_long_values_truncated_at_80x24() {
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

        // Very long values should not appear in full
        assert!(
            !cell_str.contains(&"x".repeat(100)),
            "long details should be truncated"
        );
        assert!(
            !cell_str.contains(&"y".repeat(50)),
            "long tool name should be truncated"
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
            cell_str.contains("Conversation"),
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
            cell_str.contains("Conversation"),
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
            cell_str.contains("Conversation"),
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

    #[test]
    fn test_render_permission_modal_truncates_long_values() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let (tx, _rx) = tokio::sync::oneshot::channel();
        let very_long_details = "a".repeat(200);
        let very_long_tool = "b".repeat(100);
        let reason = hackpi_guardrails::GuardReason {
            guard: hackpi_guardrails::GuardType::CommandGate,
            tool: very_long_tool,
            details: very_long_details,
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
            cell_str.contains("Permission Required"),
            "modal should show title"
        );
        assert!(
            cell_str.contains("Allow once"),
            "modal should show Allow once option"
        );
        // The long values should be truncated (there should be no 200 consecutive 'a's)
        assert!(
            !cell_str.contains(&"a".repeat(100)),
            "long details should be truncated"
        );
        assert!(
            !cell_str.contains(&"b".repeat(50)),
            "long tool name should be truncated"
        );
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
