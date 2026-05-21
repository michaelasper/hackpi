use hackpi_guardrails::{GuardReason, PermissionDecision};

/// Represents a pending permission prompt awaiting user decision.
pub struct PermissionPrompt {
    pub id: u64,
    pub reason: GuardReason,
    pub response: Option<tokio::sync::oneshot::Sender<PermissionDecision>>,
    /// When `true`, the user has pressed [4] once and must press it again
    /// to confirm `AlwaysAllow`. Set to `false` on any other key press.
    pub confirming_always_allow: bool,
}

impl std::fmt::Debug for PermissionPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionPrompt")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .field("response", &self.response.as_ref().map(|_| "Sender<..>"))
            .field("confirming_always_allow", &self.confirming_always_allow)
            .finish()
    }
}

/// Map a key character to a `PermissionDecision`, matching the key bindings
/// used in the TUI event loop when a permission prompt is active.
///
/// Note: This does NOT handle the two-step confirmation for Always allow
/// (key '4'), which is implemented in the event loop in main.rs.
pub fn permission_decision_from_key(c: char) -> Option<PermissionDecision> {
    match c {
        '1' => Some(PermissionDecision::AllowOnce),
        '2' => Some(PermissionDecision::AllowSession),
        '3' => Some(PermissionDecision::Deny),
        '4' => Some(PermissionDecision::AlwaysAllow),
        '5' => Some(PermissionDecision::AlwaysDeny),
        _ => None,
    }
}
