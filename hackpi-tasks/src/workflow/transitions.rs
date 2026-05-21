use serde::{Deserialize, Serialize};

// ── Transition ──────────────────────────────────────────────────────────────

/// A single state transition rule: from a given state, which target states are
/// allowed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Transition {
    /// Source state name.
    pub from: String,
    /// Allowed target state names.
    pub to: Vec<String>,
}
