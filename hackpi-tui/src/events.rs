use hackpi_core::tools::ToolResult;
use hackpi_core::types::Usage;
use hackpi_guardrails::{GuardReason, PermissionDecision};

#[derive(Debug)]
pub enum TuiEvent {
    Submit(String),
    StreamChunk(String),
    ToolCall {
        id: String,
        name: String,
    },
    ToolResult {
        id: String,
        result: ToolResult,
    },
    Error(String),
    Usage(Usage),
    Done,
    PermissionRequest {
        id: u64,
        reason: GuardReason,
        response: tokio::sync::oneshot::Sender<PermissionDecision>,
    },
}
