use crate::app::{App, AppView};
use crate::theme::{format_task_state, priority_label, priority_style, task_state_style, Theme};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Canonical ordering for task states in the board.
/// States not in this list sort last, preserving their string order.
const STATE_ORDER: &[&str] = &[
    "backlog",
    "todo",
    "in_progress",
    "in_review",
    "staged",
    "ready",
    "blocked",
    "done",
    "cancelled",
];

/// Return the sort rank for a task state key.
/// Known states get their index; unknown states sort after all known ones.
fn state_rank(state: &str) -> usize {
    STATE_ORDER
        .iter()
        .position(|&s| s == state)
        .unwrap_or(STATE_ORDER.len())
}

/// Truncate a string to fit within `max_display_width` terminal columns,
/// appending "…" if truncated. Measures by display width (CJK-aware).
fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut width = 0;
    let mut end = 0;
    for (i, c) in s.char_indices() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if width + cw > max_width {
            break;
        }
        width += cw;
        end = i + c.len_utf8();
    }
    if end < s.len() {
        // Need to truncate — try to fit ellipsis (width 1)
        let ellipsis_width = 1;
        let mut trunc_end = end;
        let mut trunc_width = width;
        while trunc_width + ellipsis_width > max_width && trunc_end > 0 {
            let prev = s[..trunc_end]
                .char_indices()
                .next_back()
                .map(|(i, c)| (i, c.len_utf8()))
                .unwrap();
            let cw = unicode_width::UnicodeWidthChar::width(s[prev.0..].chars().next().unwrap())
                .unwrap_or(0);
            trunc_end = prev.0;
            trunc_width -= cw;
        }
        format!("{}…", &s[..trunc_end])
    } else {
        s.to_string()
    }
}

/// Format a DateTime<Utc> as a local time string (YYYY-MM-DD HH:MM).
pub(crate) fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*dt);
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Render the task board list view, grouped by state in canonical order.
pub(crate) fn render_task_board(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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

    // Sort tasks by canonical state order, then by original position.
    // Build an index-orderable list: (sort_key, original_idx, &Task)
    let mut indexed_tasks: Vec<(usize, usize, &hackpi_tasks::Task)> = app
        .task_list_cache
        .iter()
        .enumerate()
        .map(|(i, t)| (state_rank(&t.state), i, t))
        .collect();
    indexed_tasks.sort_by_key(|(rank, idx, _)| (*rank, *idx));

    // Build state groups from the sorted list.
    // Each group is (state_key, [(task_index, &Task)]).
    let mut groups: Vec<(String, Vec<(usize, &hackpi_tasks::Task)>)> = Vec::new();
    for (_rank, original_idx, task) in &indexed_tasks {
        let state_key = &task.state;
        if let Some(last) = groups.last_mut() {
            if last.0 == *state_key {
                last.1.push((*original_idx, *task));
                continue;
            }
        }
        groups.push((state_key.clone(), vec![(*original_idx, *task)]));
    }

    // Compute available width for header filler and row truncation
    let area_width = area.width as usize;

    // Fixed-width prefix per row: "  " (cursor) + ID + " " + "[State] " + " "
    // We compute the actual prefix width per task to truncate the title.
    // Minimum budget for title: we always show cursor + ID + badge, truncate title.

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

            // Compute the fixed prefix width: cursor + ID + space + [badge] + space
            let state_label = format_task_state(&task.state);
            let id_text = format!("{} ", task.id);
            let badge_text = format!("[{state_label}] ");

            let prefix_display_width = UnicodeWidthStr::width(cursor)
                + UnicodeWidthStr::width(id_text.as_str())
                + UnicodeWidthStr::width(badge_text.as_str());

            // Title budget: area_width minus prefix minus 1 trailing space margin
            let title_budget = area_width.saturating_sub(prefix_display_width);

            let truncated_title = truncate_to_display_width(&task.title, title_budget);

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
            line_spans.push(Span::styled(id_text, theme.fg_emphasis));

            // State badge (human-readable)
            line_spans.push(Span::styled(badge_text, state_style));

            // Title (truncated to fit)
            line_spans.push(Span::styled(
                truncated_title,
                if is_selected {
                    theme.fg_emphasis
                } else {
                    theme.fg_default
                },
            ));

            items.push(ListItem::new(Line::from(line_spans)));

            // Show blocked_by as indented sub-entries, truncating long lines
            if !task.blocked_by.is_empty() {
                let blocked_prefix = "      ⬑ blocked by ";
                let blocked_prefix_width = UnicodeWidthStr::width(blocked_prefix);
                for blocker_id in &task.blocked_by {
                    let full_line = format!("{blocked_prefix}{blocker_id}");
                    let full_width = UnicodeWidthStr::width(full_line.as_str());
                    let display_line = if full_width > area_width {
                        let budget = area_width.saturating_sub(blocked_prefix_width);
                        format!(
                            "{blocked_prefix}{}",
                            truncate_to_display_width(blocker_id, budget)
                        )
                    } else {
                        full_line
                    };
                    items.push(ListItem::new(Line::from(Span::styled(
                        display_line,
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
pub(crate) fn render_task_graph(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut lines: Vec<Line> = Vec::new();

    let area_width = area.width as usize;

    // Title
    lines.push(Line::from(Span::styled(
        " Task Dependencies",
        theme.fg_emphasis.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let selected_task = app.task_list_cache.get(app.selected_task_idx);

    if let Some(selected) = selected_task {
        // Show the selected task as the focal point — truncate title if needed
        let state_label = format_task_state(&selected.state);
        let sel_prefix = " Selected: ";
        let sel_prefix_w = UnicodeWidthStr::width(sel_prefix)
            + UnicodeWidthStr::width(format!("{} ", selected.id).as_str())
            + UnicodeWidthStr::width(format!("[{}] ", state_label).as_str());
        let title_budget = area_width.saturating_sub(sel_prefix_w);
        let display_title = truncate_to_display_width(&selected.title, title_budget);

        lines.push(Line::from(vec![
            Span::styled(sel_prefix, theme.fg_muted),
            Span::styled(format!("{} ", selected.id), theme.fg_emphasis),
            Span::styled(
                format!("[{}] ", state_label),
                task_state_style(&selected.state, theme),
            ),
            Span::styled(display_title, theme.fg_default),
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
                        let bt_prefix = "   ⬑ ";
                        let bt_prefix_w = UnicodeWidthStr::width(bt_prefix)
                            + UnicodeWidthStr::width(format!("{} ", bt.id).as_str())
                            + UnicodeWidthStr::width(format!("[{}] ", bt_label).as_str());
                        let bt_budget = area_width.saturating_sub(bt_prefix_w);
                        let bt_title = truncate_to_display_width(&bt.title, bt_budget);

                        lines.push(Line::from(vec![
                            Span::styled(bt_prefix, theme.status_error),
                            Span::styled(format!("{} ", bt.id), theme.fg_emphasis),
                            Span::styled(
                                format!("[{}] ", bt_label),
                                task_state_style(&bt.state, theme),
                            ),
                            Span::styled(bt_title, theme.fg_default),
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
                        let bt_prefix = "   ⤳ ";
                        let bt_prefix_w = UnicodeWidthStr::width(bt_prefix)
                            + UnicodeWidthStr::width(format!("{} ", bt.id).as_str())
                            + UnicodeWidthStr::width(format!("[{}] ", bt_label).as_str());
                        let bt_budget = area_width.saturating_sub(bt_prefix_w);
                        let bt_title = truncate_to_display_width(&bt.title, bt_budget);

                        lines.push(Line::from(vec![
                            Span::styled(bt_prefix, theme.status_info),
                            Span::styled(format!("{} ", bt.id), theme.fg_emphasis),
                            Span::styled(
                                format!("[{}] ", bt_label),
                                task_state_style(&bt.state, theme),
                            ),
                            Span::styled(bt_title, theme.fg_default),
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
            " No tasks loaded yet.",
            theme.fg_muted,
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Press Tab to go to Tasks, then 'n' to create one.",
            theme.fg_muted,
        )));
        lines.push(Line::from(Span::styled(
            " Or type /task create <title> in the conversation.",
            theme.fg_muted,
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " No task selected.",
            theme.fg_muted,
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Use Up/Down to select a task, then Enter to view dependencies.",
            theme.fg_muted,
        )));
        lines.push(Line::from(Span::styled(
            " Press Tab to switch to the task board.",
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

/// Render the task detail view showing full task information.
pub(crate) fn render_task_detail(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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

    let area_width = area.width as usize;

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

    // Labels field — truncation to area width
    let label_prefix = "  Labels:      ";
    let label_prefix_w = UnicodeWidthStr::width(label_prefix);
    let labels_display = if task.labels.is_empty() {
        em_dash.to_string()
    } else {
        task.labels.join(", ")
    };
    let labels_budget = area_width.saturating_sub(label_prefix_w);
    let labels_text = truncate_to_display_width(&labels_display, labels_budget);
    lines.push(Line::from(vec![
        Span::styled(label_prefix, theme.fg_muted),
        Span::styled(labels_text, theme.fg_default),
    ]));

    // Blocked by field — continuation rows if needed
    let blocked_by_prefix = "  Blocked by:  ";
    let blocked_by_cont = "                "; // 16 chars continuation indent
    if app.task_detail_blocked_by.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(blocked_by_prefix, theme.fg_muted),
            Span::styled(em_dash.to_string(), theme.fg_muted),
        ]));
    } else {
        let ids: Vec<&str> = app
            .task_detail_blocked_by
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        render_continuation_list(
            &mut lines,
            blocked_by_prefix,
            blocked_by_cont,
            &ids,
            theme.fg_muted,
            theme.status_error,
            area_width,
        );
    }

    // Blocking field — continuation rows if needed
    let blocking_prefix = "  Blocking:    ";
    let blocking_cont = "                ";
    if app.task_detail_blocking.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(blocking_prefix, theme.fg_muted),
            Span::styled(em_dash.to_string(), theme.fg_muted),
        ]));
    } else {
        let ids: Vec<&str> = app
            .task_detail_blocking
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        render_continuation_list(
            &mut lines,
            blocking_prefix,
            blocking_cont,
            &ids,
            theme.fg_muted,
            theme.status_warning,
            area_width,
        );
    }

    lines.push(Line::from(""));

    // Description section — wrap with Paragraph
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

/// Render a comma-separated list of IDs across continuation rows.
/// First line uses `prefix`, subsequent lines use `cont_indent`.
/// Values are comma-separated, wrapping to `area_width`.
fn render_continuation_list(
    lines: &mut Vec<Line<'static>>,
    prefix: &'static str,
    cont_indent: &'static str,
    ids: &[&str],
    prefix_style: Style,
    value_style: Style,
    area_width: usize,
) {
    let prefix_w = UnicodeWidthStr::width(prefix);
    let cont_w = UnicodeWidthStr::width(cont_indent);

    // Build comma-separated string and see if it fits on one line
    let joined = ids.join(", ");
    let joined_w = UnicodeWidthStr::width(joined.as_str());

    if prefix_w + joined_w <= area_width {
        // Fits on one line
        lines.push(Line::from(vec![
            Span::styled(prefix, prefix_style),
            Span::styled(joined, value_style),
        ]));
        return;
    }

    // Need continuation rows. Emit items one at a time, wrapping as needed.
    let mut first_line = true;
    let mut current_line_ids: Vec<&str> = Vec::new();
    let mut current_line_w: usize = 0;

    let indent_w = if first_line { prefix_w } else { cont_w };
    let available = area_width.saturating_sub(indent_w);

    for id in ids {
        let id_w = UnicodeWidthStr::width(*id);
        let separator_w = if current_line_ids.is_empty() {
            0
        } else {
            2 // ", "
        };

        if !current_line_ids.is_empty() && current_line_w + separator_w + id_w > available {
            // Flush current line
            let text = current_line_ids.join(", ");
            if first_line {
                lines.push(Line::from(vec![
                    Span::styled(prefix, prefix_style),
                    Span::styled(text, value_style),
                ]));
                first_line = false;
            } else {
                lines.push(Line::from(vec![
                    Span::styled(cont_indent, prefix_style),
                    Span::styled(text, value_style),
                ]));
            }
            current_line_ids.clear();
            current_line_w = 0;

            // Recompute available for continuation lines
            // (cont_indent is constant for non-first lines)
        }

        current_line_w += if current_line_ids.is_empty() {
            id_w
        } else {
            separator_w + id_w
        };
        current_line_ids.push(id);
    }

    // Flush remaining
    if !current_line_ids.is_empty() {
        let text = current_line_ids.join(", ");
        if first_line {
            lines.push(Line::from(vec![
                Span::styled(prefix, prefix_style),
                Span::styled(text, value_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::styled(cont_indent, prefix_style),
                Span::styled(text, value_style),
            ]));
        }
    }
}
