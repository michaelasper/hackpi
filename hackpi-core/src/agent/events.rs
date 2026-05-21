use tokio::sync::mpsc;

use super::state::{Agent, AgentEvent};

impl Agent {
    /// Check if the stop reason indicates conversation should end.
    /// Returns `true` if the turn should stop.
    pub(crate) fn handle_step_stop_reason(
        stop_reason: &Option<String>,
        tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> bool {
        let should_stop = matches!(stop_reason, Some(s) if s == "end_turn" || s == "stop");
        if should_stop {
            tx.send(AgentEvent::Done).ok();
        }
        should_stop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_step_stop_reason_end_turn_returns_true() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("end_turn".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_stop_returns_true() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("stop".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_other_reason_returns_false() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason = Some("tool_use".to_string());
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_none_returns_false() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let stop_reason: Option<String> = None;
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));
    }

    #[test]
    fn test_handle_step_stop_reason_end_turn_sends_done_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("end_turn".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));

        // Should have received a Done event
        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(_) => panic!("expected Done event"),
            Err(_) => panic!("expected Done event, got empty channel"),
        }
    }

    #[test]
    fn test_handle_step_stop_reason_stop_sends_done_event() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("stop".to_string());
        assert!(Agent::handle_step_stop_reason(&stop_reason, &tx));

        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(_) => panic!("expected Done event"),
            Err(_) => panic!("expected Done event, got empty channel"),
        }
    }

    #[test]
    fn test_handle_step_stop_reason_other_does_not_send_done() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let stop_reason = Some("tool_use".to_string());
        assert!(!Agent::handle_step_stop_reason(&stop_reason, &tx));

        assert!(
            rx.try_recv().is_err(),
            "should not send any event for tool_use stop reason"
        );
    }
}
