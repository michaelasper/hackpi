use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct InputHandler {
    pub buffer: String,
    pub cursor: usize,
    last_submitted: Option<String>,
}

impl Default for InputHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl InputHandler {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            last_submitted: None,
        }
    }

    /// Convert the char-indexed cursor to a byte offset into `self.buffer`.
    /// If the cursor is at or past the end of the char sequence, returns
    /// the byte length of the buffer.
    fn byte_pos(&self) -> usize {
        self.buffer
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.buffer.len())
    }

    /// Return the number of Unicode scalar values in the buffer.
    fn char_count(&self) -> usize {
        self.buffer.chars().count()
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.last_submitted = None;
        match key.code {
            KeyCode::Enter => {
                if key.modifiers == KeyModifiers::SHIFT {
                    let pos = self.byte_pos();
                    self.buffer.insert(pos, '\n');
                    self.cursor += 1;
                } else {
                    let submitted = self.buffer.trim().to_string();
                    if !submitted.is_empty() {
                        self.last_submitted = Some(submitted);
                    }
                    self.buffer.clear();
                    self.cursor = 0;
                }
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                let pos = self.byte_pos();
                self.buffer.remove(pos);
            }
            KeyCode::Delete if self.cursor < self.char_count() => {
                let pos = self.byte_pos();
                self.buffer.remove(pos);
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right if self.cursor < self.char_count() => {
                self.cursor += 1;
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.char_count();
            }
            KeyCode::Char(ch) => {
                let pos = self.byte_pos();
                self.buffer.insert(pos, ch);
                self.cursor += 1;
            }
            _ => {}
        }
    }

    pub fn last_submitted(&mut self) -> Option<String> {
        self.last_submitted.take()
    }

    /// Set the buffer to the given text and mark it as submitted.
    /// Used by autocomplete to insert and submit a selected command.
    pub fn set_submit(&mut self, text: String) {
        self.last_submitted = Some(text.clone());
        self.buffer = text;
        self.cursor = self.char_count();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_submit_sets_buffer_and_submitted() {
        let mut input = InputHandler::new();
        input.set_submit("/help".to_string());
        assert_eq!(input.buffer, "/help");
        assert_eq!(input.cursor, 5);
        assert_eq!(input.last_submitted(), Some("/help".to_string()));
        // After taking, last_submitted should be None
        assert_eq!(input.last_submitted(), None);
    }

    #[test]
    fn test_set_submit_with_empty_string() {
        let mut input = InputHandler::new();
        input.set_submit(String::new());
        assert_eq!(input.buffer, "");
        // last_submitted should still be Some even for empty string
        assert_eq!(input.last_submitted(), Some(String::new()));
    }

    #[test]
    fn test_set_submit_clears_previous_submission() {
        let mut input = InputHandler::new();
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        input.set_submit("/clear".to_string());
        assert_eq!(input.buffer, "/clear");
        assert_eq!(input.cursor, 6);
        assert_eq!(input.last_submitted(), Some("/clear".to_string()));
    }

    // --- Non-ASCII / Unicode tests ---

    #[test]
    fn test_insert_cjk_characters() {
        let mut input = InputHandler::new();
        // Insert a CJK character (3 bytes in UTF-8)
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "中");
        assert_eq!(
            input.cursor, 1,
            "cursor should be 1 char after inserting one char"
        );

        // Insert another CJK character
        input.handle_key(KeyEvent::new(KeyCode::Char('国'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "中国");
        assert_eq!(
            input.cursor, 2,
            "cursor should be 2 chars after inserting two chars"
        );
    }

    #[test]
    fn test_left_right_navigation_with_cjk() {
        let mut input = InputHandler::new();
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "a中b");
        assert_eq!(input.cursor, 3);

        // Move left 3 times to get to start
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 2, "left from pos 3 -> pos 2 (past 'b')");
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 1, "left from pos 2 -> pos 1 (past '中')");
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 0, "left from pos 1 -> pos 0 (past 'a')");

        // Move right back
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 1, "right from pos 0 -> pos 1");
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 2, "right from pos 1 -> pos 2");
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 3, "right from pos 2 -> pos 3");
    }

    #[test]
    fn test_backspace_cjk() {
        let mut input = InputHandler::new();
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('国'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "a中国");

        // Backspace should remove '国'
        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.buffer, "a中", "backspace should remove last CJK char");
        assert_eq!(input.cursor, 2, "cursor should be at char index 2");

        // Backspace again should remove '中'
        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.buffer, "a", "backspace should remove middle CJK char");
        assert_eq!(input.cursor, 1, "cursor should be at char index 1");

        // Backspace again should remove 'a'
        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.buffer, "", "backspace should remove last ASCII char");
        assert_eq!(input.cursor, 0, "cursor should be at char index 0");
    }

    #[test]
    fn test_delete_cjk() {
        let mut input = InputHandler::new();
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "a中b");

        // Move cursor to pos 1 (before 中), then delete should remove 中
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 2);
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 1);

        input.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(
            input.buffer, "ab",
            "delete should remove the CJK char at cursor"
        );
        assert_eq!(
            input.cursor, 1,
            "cursor should stay at same position after delete"
        );
    }

    #[test]
    fn test_mixed_ascii_and_unicode() {
        let mut input = InputHandler::new();
        // Type: "héllo wörld 🔥"
        for ch in "héllo wörld 🔥".chars() {
            input.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(input.buffer, "héllo wörld 🔥");

        // Navigate to end
        input.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(input.cursor, 13, "total chars: h(1) é(2) l(3) l(4) o(5) ' '(6) w(7) ö(8) r(9) l(10) d(11) ' '(12) 🔥(13)");

        // Navigate home
        input.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        assert_eq!(input.cursor, 0);

        // Navigate right past é
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 1);
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 2);
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn test_insert_in_middle_with_cjk() {
        let mut input = InputHandler::new();
        // Start with "ab"
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));

        // Move cursor left by 1
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 1);

        // Insert CJK character in middle → "a中b"
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        assert_eq!(input.buffer, "a中b", "CJK char inserted in middle of ASCII");
        assert_eq!(
            input.cursor, 2,
            "cursor after inserting at pos 1 should be pos 2"
        );
    }

    #[test]
    fn test_shift_enter_with_cjk() {
        let mut input = InputHandler::new();
        // Type "a中\n国b" using Shift+Enter for the newline
        input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('中'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        input.handle_key(KeyEvent::new(KeyCode::Char('国'), KeyModifiers::NONE));
        input.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(
            input.buffer, "a中\n国b",
            "Shift+Enter should insert newline in multi-byte context"
        );
    }

    #[test]
    fn test_cursor_clamping_on_empty_with_cjk() {
        let mut input = InputHandler::new();
        // Left on empty buffer should stay at 0
        input.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.cursor, 0);

        // Right on empty buffer should stay at 0
        input.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(input.cursor, 0);

        // Delete on empty buffer should be no-op
        input.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(input.buffer, "");
        assert_eq!(input.cursor, 0);

        // Backspace on empty buffer should be no-op
        input.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(input.buffer, "");
        assert_eq!(input.cursor, 0);
    }
}
