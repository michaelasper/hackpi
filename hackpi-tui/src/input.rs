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

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.last_submitted = None;
        match key.code {
            KeyCode::Enter => {
                if key.modifiers == KeyModifiers::SHIFT {
                    self.buffer.insert(self.cursor, '\n');
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
                let pos = self.cursor - 1;
                self.buffer.remove(pos);
                self.cursor = pos;
            }
            KeyCode::Delete if self.cursor < self.buffer.len() => {
                self.buffer.remove(self.cursor);
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right if self.cursor < self.buffer.len() => {
                self.cursor += 1;
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
            }
            KeyCode::Char(ch) => {
                self.buffer.insert(self.cursor, ch);
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
        self.cursor = self.buffer.len();
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
}
