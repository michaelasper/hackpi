use crate::app::{App, AppView};
use crate::theme::{format_task_state, priority_label, priority_style, task_state_style, Theme};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Format a DateTime<Utc> as a local time string (YYYY-MM-DD HH:MM).
pub(crate) fn format_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> String {
    let local: chrono::DateTime<chrono::Local> = chrono::DateTime::from(*dt);
    local.format("%Y-%m-%d %H:%M").to_string()
}

/// Render the task board list view, grouped by state with counts.
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
pub(crate) fn render_task_graph(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
