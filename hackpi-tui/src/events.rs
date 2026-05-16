use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;

#[derive(Debug, Clone)]
pub enum TuiEvent {
    Submit(String),
    StreamChunk(String),
    ToolCall { id: String, name: String },
    ToolResult { id: String, result: ToolResult },
    Error(String),
    Usage(Usage),
    Done,
}
