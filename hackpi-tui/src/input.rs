use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct InputHandler {
    pub buffer: String,
    pub cursor: usize,
}

impl InputHandler {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Enter => {
                if key.modifiers == KeyModifiers::SHIFT {
                    self.buffer.insert(self.cursor, '\n');
                    self.cursor += 1;
                    None
                } else {
                    let submitted = self.buffer.trim().to_string();
                    if submitted.is_empty() {
                        return None;
                    }
                    self.buffer.clear();
                    self.cursor = 0;
                    Some(submitted)
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let pos = self.cursor - 1;
                    self.buffer.remove(pos);
                    self.cursor = pos;
                }
                None
            }
            KeyCode::Delete => {
                if self.cursor < self.buffer.len() {
                    self.buffer.remove(self.cursor);
                }
                None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            KeyCode::Right => {
                if self.cursor < self.buffer.len() {
                    self.cursor += 1;
                }
                None
            }
            KeyCode::Home => {
                self.cursor = 0;
                None
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
                None
            }
            KeyCode::Char(ch) => {
                self.buffer.insert(self.cursor, ch);
                self.cursor += 1;
                None
            }
            _ => None,
        }
    }
}
