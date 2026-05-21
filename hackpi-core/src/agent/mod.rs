mod api;
mod conversation;
mod events;
mod state;
mod tool_dispatch;

// Re-export public API
pub use state::{Agent, AgentEvent};

use crate::types::{ContentBlock, Message, Role};
use state::MAX_TURNS;

impl Agent {
    pub async fn run(
        &self,
        user_message: &str,
        conversation: &mut Vec<Message>,
        tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        signal: tokio::sync::watch::Receiver<bool>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        conversation.push(Message {
            role: Role::User,
            content: vec![ContentBlock::text(user_message)],
        });

        for _turn in 0..MAX_TURNS {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                tx.send(AgentEvent::Done).ok();
                return;
            }

            let (api_tx, mut api_rx) = tokio::sync::mpsc::unbounded_channel();

            let send_result = self
                .api
                .send_messages(
                    conversation,
                    &self.tools.all_schemas(),
                    &self.system_prompt,
                    api_tx,
                )
                .await;

            if let Err(e) = send_result {
                tx.send(AgentEvent::Error(format!("API error: {e}"))).ok();
                break;
            }

            // Process SSE events from the API stream
            let events = match Self::process_sse_events(&mut api_rx, &tx, &cancelled).await {
                Some(events) => events,
                None => return,
            };

            // Build assistant message from accumulated text and tool calls.
            // Text blocks are placed first, followed by ToolUse content blocks,
            // so the conversation history correctly precedes tool_result blocks.
            Self::build_assistant_message(conversation, &events.text, &events.pending_tool_calls);

            // Report usage if available
            if let Some(u) = events.usage {
                tx.send(AgentEvent::Usage(u)).ok();
            }

            // If no tool calls, we're done
            if events.pending_tool_calls.is_empty() {
                tx.send(AgentEvent::Done).ok();
                return;
            }

            // Execute pending tool calls
            let tool_results = match self
                .execute_pending_tool_calls(&events.pending_tool_calls, &tx, &signal)
                .await
            {
                Some(results) => results,
                None => return,
            };

            // Push tool results back to conversation
            if !tool_results.is_empty() {
                conversation.push(Message {
                    role: Role::User,
                    content: tool_results,
                });
            }

            // Check if we should stop based on stop reason
            if Self::handle_step_stop_reason(&events.stop_reason, &tx) {
                return;
            }
        }

        tx.send(AgentEvent::TextChunk(
            "\n\n[Turn limit reached. Starting fresh on your next request.]".into(),
        ))
        .ok();
        tx.send(AgentEvent::Done).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiClient;
    use crate::tools::ToolRegistry;
    use crate::types::ApiConfig;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    // ── run() cancellation tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_run_returns_immediately_when_cancelled_true() {
        // When `cancelled` is true at the start of `run()`, the agent must
        // send `AgentEvent::Done` and return immediately without making any
        // API calls. This covers the core of COR-183: without a reset, a
        // previous Ctrl+C interrupt latches and kills the next generation.
        let api = ApiClient::new(ApiConfig::default()).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let (tx, mut rx) = mpsc::unbounded_channel();
        let (_signal_tx, signal_rx) = tokio::sync::watch::channel(false);
        let cancelled = Arc::new(AtomicBool::new(true));
        let mut conversation = Vec::new();

        agent
            .run("hello", &mut conversation, tx, signal_rx, cancelled)
            .await;

        // Should have sent exactly one Done event
        match rx.try_recv() {
            Ok(AgentEvent::Done) => {} // expected
            Ok(other) => panic!("expected Done, got {other:?}"),
            Err(e) => panic!("expected Done event, got empty channel: {e:?}"),
        }

        // No more events should be queued
        assert!(
            rx.try_recv().is_err(),
            "should have exactly one event (Done)"
        );

        // User message is pushed to conversation before the cancel check,
        // so conversation should have 1 entry. This documents the current
        // behavior — the user input is recorded even on cancellation.
        assert_eq!(
            conversation.len(),
            1,
            "conversation should contain user message"
        );
        assert!(
            matches!(conversation[0].role, Role::User),
            "the message should have User role"
        );
    }

    #[tokio::test]
    async fn test_run_does_not_exit_immediately_when_cancelled_reset() {
        // Regression test for COR-183: after resetting the cancelled flag,
        // a subsequent `run()` call must not exit immediately. Uses a
        // wiremock server to verify the agent actually makes an API call.
        let mock_server = wiremock::MockServer::start().await;

        // Mock a streaming response with a tool_use stop reason
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_string(
                    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"content\":[]}}\n\n\
                     data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
                     data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"text\":\"Hello!\"}}\n\n\
                     data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
                     data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":5}}\n\n\
                     data: [DONE]\n\n",
                ),
            )
            .mount(&mock_server)
            .await;

        let api_config = ApiConfig {
            endpoint: format!("{}/v1/messages", mock_server.uri()),
            model: "ds4".into(),
            max_tokens: 100,
            temperature: 0.0,
        };
        let api = ApiClient::new(api_config).unwrap();
        let tools = Arc::new(ToolRegistry::new());
        let agent = Agent::new(api, tools, "system".into(), PathBuf::from("/tmp"));

        let cancelled = Arc::new(AtomicBool::new(true));

        // First run with cancelled=true — should exit immediately
        {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (_stx, srx) = tokio::sync::watch::channel(false);
            let mut conversation = Vec::new();

            agent
                .run("first", &mut conversation, tx, srx, Arc::clone(&cancelled))
                .await;

            match rx.try_recv() {
                Ok(AgentEvent::Done) => {} // expected
                Ok(other) => panic!("expected Done, got {other:?}"),
                Err(e) => panic!("expected Done event: {e:?}"),
            }
        }

        // Reset cancelled flag (mirrors the fix applied in main.rs)
        cancelled.store(false, std::sync::atomic::Ordering::SeqCst);

        // Second run with cancelled=false — must NOT exit immediately.
        // The agent should reach the API call and get a response.
        {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let (_stx, srx) = tokio::sync::watch::channel(false);
            let mut conversation = Vec::new();

            agent
                .run("second", &mut conversation, tx, srx, Arc::clone(&cancelled))
                .await;

            // Should NOT be Done — the agent should have processed the
            // streaming response, which includes "Hello!" text and an
            // end_turn stop reason.
            let mut found_hello = false;
            let mut found_done = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    AgentEvent::TextChunk(text) if text == "Hello!" => {
                        found_hello = true;
                    }
                    AgentEvent::Done => {
                        found_done = true;
                    }
                    AgentEvent::Usage(usage) => {
                        assert_eq!(usage.input_tokens, 10);
                        assert_eq!(usage.output_tokens, 5);
                    }
                    _ => {} // other events are fine
                }
            }

            assert!(
                found_hello,
                "should have received 'Hello!' text from mock server"
            );
            assert!(found_done, "should have received Done event at end");
        }
    }

    // ── AtomicBool reset semantics ────────────────────────────────────

    #[test]
    fn test_atomic_bool_reset_makes_load_return_false() {
        // Verify that storing true then false on an AtomicBool correctly
        // resets the flag — this is the core mechanism of the COR-183 fix.
        let flag = AtomicBool::new(true);
        assert!(
            flag.load(std::sync::atomic::Ordering::SeqCst),
            "should start as true"
        );

        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(
            !flag.load(std::sync::atomic::Ordering::SeqCst),
            "should be false after reset"
        );
    }
}
