use crate::app::App;
use crate::theme::Theme;
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

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

pub(crate) fn render_input(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
