use crate::events::TuiEvent;
use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;
use std::collections::VecDeque;

pub enum AppState {
    Resting,
    Generating,
    Interrupted,
}

pub struct ConversationEntry {
    pub role: String,
    pub text: String,
    pub tool_calls: Vec<ToolCallDisplay>,
}

pub struct ToolCallDisplay {
    pub id: String,
    pub name: String,
    pub status: ToolCallStatus,
}

pub enum ToolCallStatus {
    Running,
    Done(ToolResult),
}

pub struct App {
    pub state: AppState,
    pub input: String,
    pub conversation: VecDeque<ConversationEntry>,
    pub scroll_offset: usize,
    pub usage: Option<Usage>,
    pub status_message: String,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::Resting,
            input: String::new(),
            conversation: VecDeque::new(),
            scroll_offset: 0,
            usage: None,
            status_message: String::new(),
        }
    }

    pub fn handle_event(&mut self, event: TuiEvent) {
        match event {
            TuiEvent::Submit(text) => {
                self.conversation.push_back(ConversationEntry {
                    role: "user".into(),
                    text,
                    tool_calls: Vec::new(),
                });
                self.state = AppState::Generating;
                self.scroll_offset = 0;
            }
            TuiEvent::StreamChunk(chunk) => {
                if let Some(entry) = self.conversation.back_mut() {
                    if entry.role == "assistant" {
                        entry.text.push_str(&chunk);
                    }
                } else {
                    self.conversation.push_back(ConversationEntry {
                        role: "assistant".into(),
                        text: chunk,
                        tool_calls: Vec::new(),
                    });
                }
            }
            TuiEvent::ToolCall { id, name } => {
                if let Some(entry) = self.conversation.back_mut() {
                    if entry.role != "assistant" {
                        self.conversation.push_back(ConversationEntry {
                            role: "assistant".into(),
                            text: String::new(),
                            tool_calls: Vec::new(),
                        });
                    }
                    if let Some(entry) = self.conversation.back_mut() {
                        entry.tool_calls.push(ToolCallDisplay {
                            id,
                            name,
                            status: ToolCallStatus::Running,
                        });
                    }
                }
            }
            TuiEvent::ToolDelta { id: _, delta: _ } => {}
            TuiEvent::ToolResult { id, result } => {
                if let Some(entry) = self.conversation.back_mut() {
                    for tc in &mut entry.tool_calls {
                        if tc.id == id {
                            tc.status = ToolCallStatus::Done(result);
                            break;
                        }
                    }
                }
            }
            TuiEvent::Usage(usage) => {
                self.usage = Some(usage);
            }
            TuiEvent::Error(err) => {
                self.status_message = err;
                self.state = AppState::Resting;
            }
            TuiEvent::Done => {
                self.state = AppState::Resting;
            }
        }
    }

    pub fn clear(&mut self) {
        self.conversation.clear();
        self.input.clear();
        self.usage = None;
        self.scroll_offset = 0;
    }
}
