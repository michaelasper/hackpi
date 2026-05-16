# [Architecture] - [MEDIUM] - Conversation history is not persisted and recreated per API call

**Labels:** `architecture`, `priority-medium`, `missing-feature`
**Status:** RESOLVED

**Fix:** Moved `conversation_mut` from inside the submit handler to the outer scope, wrapping it in `Arc<tokio::sync::Mutex<Vec<Message>>>` to share across `tokio::spawn` tasks. Each spawn clones the Arc, locks the mutex, and passes `&mut *guard` to `Agent::run`. Conversation now persists across user turns within a session.

## Description

In `agent.rs:44-50`, the `run` method accepts `conversation: &mut Vec<Message>` but in `main.rs:151`, the caller creates `let mut conversation_mut = Vec::new();` — a fresh empty conversation vector for every user submission. This means:

1. Conversation history between user turns is lost. The agent starts with an empty history on every `Submit`.
2. The LLM has no memory of previous exchanges in the same session.
3. The `Turn limit reached` message forces the user to start over entirely.

Additionally, on app restart (Ctrl+D and relaunch), all history is lost. There's no persistence layer.

The `conversation` parameter is `&mut Vec<Message>` but the agent pushes messages into it, so the caller **could** preserve history across calls — but `main.rs` doesn't do this. The agent pushes a user message (line 51-54), assistant messages (line 147-151, 232-237), and tool results (line 238-241) into the same Vec. If the Vec were properly maintained across calls, the LLM would have full conversation context.

## Location

- `hackpi-tui/src/main.rs:151` — `let mut conversation_mut = Vec::new();` — fresh Vec per submit
- `hackpi-core/src/agent.rs:51-54` — Agent pushes user message into conversation
- `hackpi-core/src/agent.rs:147-151, 232-241` — Agent pushes assistant messages and tool results

## Impact

- **Zero conversation memory**: the LLM has no context of previous turns
- Wasted tool calls: the LLM re-reads files it already inspected
- `Turn limit reached` is effectively a session end — the user must re-explain everything
- Usage tracking (`input_tokens`/`output_tokens`) only reflects the current turn, not the session total

## Proposed Solutions

1. Move `conversation_mut` outside the event loop in `main.rs` (to the `loop` level) so it persists across submits
2. Consider adding a session-level conversation cap (e.g., trim to last N messages to stay within context window)
3. Add session persistence to disk (JSON file in workspace_root) so conversation survives restart
