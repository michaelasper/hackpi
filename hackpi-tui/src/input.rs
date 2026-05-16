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
}
