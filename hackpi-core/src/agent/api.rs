use crate::api::ApiEvent;
use crate::types::Usage;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

use super::state::{Agent, AgentEvent, PendingToolCall, SseEvents};

impl Agent {
    /// Process SSE events from the API response stream.
    /// Handles text deltas, tool call starts/stops, and message deltas.
    /// Returns `None` if cancelled via signal.
    pub(crate) async fn process_sse_events(
        api_rx: &mut mpsc::UnboundedReceiver<ApiEvent>,
        tx: &mpsc::UnboundedSender<AgentEvent>,
        cancelled: &AtomicBool,
    ) -> Option<SseEvents> {
        let mut current_text = String::new();
        let mut pending_tool_calls: Vec<PendingToolCall> = Vec::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        // Tracks whether we've received any `partial_json` delta for the
        // current tool call. When the first `partial_json` arrives we
        // discard the `{}` placeholder that `content_block_start` sets from
        // `block.input`, because the Anthropic streaming API always sends
        // `input: {}` as a placeholder and the real input via deltas.
        let mut has_seen_partial_json = false;
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;

        while let Some(event) = api_rx.recv().await {
            if cancelled.load(Ordering::Relaxed) {
                tx.send(AgentEvent::Done).ok();
                return None;
            }

            match event {
                ApiEvent::Event(evt) => match evt.event_type.as_str() {
                    "content_block_delta" => {
                        if let Some(delta) = &evt.delta {
                            if let Some(text) = &delta.text {
                                current_text.push_str(text);
                                tx.send(AgentEvent::TextChunk(text.clone())).ok();
                            }
                            if let Some(stop) = &delta.stop_reason {
                                stop_reason = Some(stop.clone());
                            }
                            // Accumulate incremental JSON fragments for tool input.
                            // The Anthropic streaming API sends tool parameters as
                            // `partial_json` deltas rather than in `content_block_start`.
                            if let Some(json_fragment) = &delta.partial_json {
                                if !has_seen_partial_json {
                                    // First partial_json: discard the `{}`
                                    // placeholder from content_block_start.
                                    has_seen_partial_json = true;
                                    current_tool_input.clear();
                                }
                                current_tool_input.push_str(json_fragment);
                            }
                        }
                    }
                    "content_block_start" => {
                        if let Some(block) = &evt.content_block {
                            if block.block_type == "tool_use" {
                                current_tool_id = block.id.clone().unwrap_or_default();
                                current_tool_name = block.name.clone().unwrap_or_default();
                                current_tool_input = String::new();
                                has_seen_partial_json = false;
                                if let Some(input) = &block.input {
                                    current_tool_input = input.to_string();
                                }
                            }
                        }
                    }
                    "content_block_stop" if !current_tool_id.is_empty() => {
                        let (input, parse_error) = match serde_json::from_str(&current_tool_input) {
                            Ok(v) => (v, None),
                            Err(e) => {
                                let err_msg = format!(
                                        "Failed to parse tool '{}' input JSON: {e}\nRaw input: {current_tool_input}",
                                        current_tool_name,
                                    );
                                (Value::Null, Some(err_msg))
                            }
                        };
                        pending_tool_calls.push(PendingToolCall {
                            id: current_tool_id.clone(),
                            name: current_tool_name.clone(),
                            input,
                            parse_error,
                        });
                        current_tool_id.clear();
                        current_tool_name.clear();
                        current_tool_input.clear();
                    }
                    "message_delta" => {
                        if let Some(delta) = &evt.delta {
                            if let Some(stop) = &delta.stop_reason {
                                stop_reason = Some(stop.clone());
                            }
                        }
                        if let Some(u) = &evt.usage {
                            usage = Some(u.clone());
                        }
                    }
                    _ => {}
                },
                ApiEvent::Error(err) => {
                    tx.send(AgentEvent::Error(err)).ok();
                }
                ApiEvent::Diagnostic(msg) => {
                    tx.send(AgentEvent::Diagnostic(msg)).ok();
                }
                ApiEvent::Done => break,
            }
        }

        Some(SseEvents {
            text: current_text,
            pending_tool_calls,
            stop_reason,
            usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create an SSE event from a raw JSON string.
    fn make_event(json: &str) -> crate::api::ApiEvent {
        let event: crate::types::StreamEvent =
            serde_json::from_str(json).expect("valid SSE event JSON");
        crate::api::ApiEvent::Event(Box::new(event))
    }

    #[tokio::test]
    async fn test_process_sse_events_accumulates_partial_json() {
        // Simulate the Anthropic streaming API sending tool input via
        // content_block_start (with empty input) followed by multiple
        // content_block_delta events with partial_json fragments.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        // Send events that mirror real Anthropic streaming for a tool call
        tx.send(make_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"write","input":{}}}"#,
        )).ok();

        // These partial_json deltas carry the actual tool parameters
        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"{\"path\":"}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"\"hello.txt\","}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"\"content\":\"Hello!\"}"}}"#,
        )).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":1}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_some(), "should return SSE events");
        let events = result.unwrap();

        assert_eq!(
            events.pending_tool_calls.len(),
            1,
            "should have one pending tool call"
        );
        let tc = &events.pending_tool_calls[0];
        assert_eq!(tc.id, "toolu_01");
        assert_eq!(tc.name, "write");
        assert!(
            tc.parse_error.is_none(),
            "should parse successfully, got error: {:?}",
            tc.parse_error
        );

        // The input should be the reconstructed JSON from partial_json fragments
        assert_eq!(tc.input["path"], "hello.txt", "path should be 'hello.txt'");
        assert_eq!(tc.input["content"], "Hello!", "content should be 'Hello!'");

        // Drain the agent events (we don't need them for this test)
        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_partial_json_write_tool_params() {
        // Regression test for COR-157: the write tool should receive
        // both "path" and "content" parameters when they come via partial_json.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        let full_input = r#"{"path":"src/main.rs","content":"fn main() {}"}"#;

        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_42","name":"write","input":{}}}"#,
        )).ok();

        // Send the entire input as one partial_json chunk (common with small inputs)
        let delta_json = format!(
            r#"{{"type":"content_block_delta","index":0,"delta":{{"partial_json":{}}}}}"#,
            serde_json::to_string(full_input).unwrap()
        );
        tx.send(make_event(&delta_json)).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert!(
            tc.parse_error.is_none(),
            "should parse cleanly: {:?}",
            tc.parse_error
        );
        assert_eq!(tc.input["path"], "src/main.rs");
        assert_eq!(tc.input["content"], "fn main() {}");

        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_git_read_via_partial_json() {
        // Regression test for COR-157: the git_read tool should receive
        // the "operation" parameter when it comes via partial_json.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_99","name":"git_read","input":{}}}"#,
        )).ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"partial_json":"{\"operat"}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"partial_json":"ion\":\"log\"}"}}"#,
        ))
        .ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert!(
            tc.parse_error.is_none(),
            "should parse cleanly: {:?}",
            tc.parse_error
        );
        assert_eq!(tc.input["operation"], "log");

        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_no_partial_json_still_works() {
        // Verify that the text delta path still works when there are no
        // partial_json events (e.g., a plain text response).
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"Hello "}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"World"}}"#,
        ))
        .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.text, "Hello World");
        assert!(events.pending_tool_calls.is_empty());

        while agent_rx.try_recv().is_ok() {}
    }

    // ── process_sse_events: cancellation ──────────────────────────────

    #[tokio::test]
    async fn test_process_sse_events_returns_none_when_cancelled() {
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"hello"}}"#,
        ))
        .ok();

        let cancelled = AtomicBool::new(true);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_none(), "should return None when cancelled");

        // Should have sent a Done event
        match agent_rx.try_recv() {
            Ok(AgentEvent::Done) => {}
            Ok(other) => panic!("expected Done, got {other:?}"),
            Err(e) => panic!("expected Done event: {e:?}"),
        }

        // Drop remaining so the buffer doesn't accumulate
        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_processes_normally_when_not_cancelled() {
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"hello"}}"#,
        ))
        .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        assert!(result.is_some(), "should return events when not cancelled");
        let events = result.unwrap();
        assert_eq!(events.text, "hello");

        while agent_rx.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn test_process_sse_events_mixed_text_and_partial_json() {
        // Verify that a response containing both text deltas and tool call
        // partial_json deltas processes both correctly.
        let (tx, mut api_rx) = mpsc::unbounded_channel::<crate::api::ApiEvent>();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentEvent>();

        // Text block
        tx.send(make_event(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ))
        .ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":0,"delta":{"text":"I'll help."}}"#,
        ))
        .ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":0}"#))
            .ok();

        // Tool call block
        tx.send(make_event(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tool_1","name":"write","input":{}}}"#,
        )).ok();

        tx.send(make_event(
            r#"{"type":"content_block_delta","index":1,"delta":{"partial_json":"{\"path\":\"x\",\"content\":\"y\"}"}}"#,
        )).ok();

        tx.send(make_event(r#"{"type":"content_block_stop","index":1}"#))
            .ok();

        tx.send(crate::api::ApiEvent::Done).ok();

        let cancelled = AtomicBool::new(false);
        let result = Agent::process_sse_events(&mut api_rx, &agent_tx, &cancelled).await;

        let events = result.expect("should return events");
        assert_eq!(events.text, "I'll help.");
        assert_eq!(events.pending_tool_calls.len(), 1);
        let tc = &events.pending_tool_calls[0];
        assert_eq!(tc.input["path"], "x");
        assert_eq!(tc.input["content"], "y");

        while agent_rx.try_recv().is_ok() {}
    }
}
