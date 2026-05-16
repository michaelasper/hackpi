# [Bugs] - [HIGH] - Empty assistant message pushed on every tool-use turn pollutes conversation

**Labels:** `bug`, `priority-high`, `performance`
**Status:** RESOLVED

**Fix:** Removed the `if turn > 0` block in `agent.rs` that pushed an empty assistant message (`ContentBlock::text("")`) before tool results. Tool results are still pushed as a User message (preserving the API contract), but no wasteful empty message precedes them.

## Description

In `agent.rs:232-237`, on every tool-use turn after the first (`if turn > 0`), an empty assistant message (`ContentBlock::text("")`) is pushed to the conversation before the tool results (which are pushed as a User message at line 239). This means:

- Turn 1: [User message, Assistant response with text + tool_calls, User message with tool_results]
- Turn 2: [..., empty Assistant message, User message with tool_results]
- Turn 3: [..., empty Assistant message, User message with tool_results]
- ...up to 25 times

Each empty assistant message costs tokens in the input context. For a maximum-turn conversation (25 turns), this adds 24 empty messages, each with serialization overhead in the API request.

The Anthropic Messages API expects tool results to be part of the assistant's response turn, not a separate user message. Pushing tool results as a user message with role=User is a structural issue — tool results should be part of the assistant's content blocks with `type: "tool_result"`.

## Location

- `hackpi-core/src/agent.rs:232-242` — Empty assistant push and tool results as User role

## Impact

- ~24 wasted messages in long conversations, inflating token usage
- API may not handle tool_results under `role: "user"` correctly (depends on server implementation)
- Increases per-request payload size unnecessarily

## Proposed Solutions

1. Add tool result content blocks (with `type: "tool_result"`) directly to the assistant message's `content` vector instead of creating a new User message
2. Remove the empty assistant message entirely — tool results should be part of the same assistant turn
3. Verify that the ds4-server `/v1/messages` endpoint expects `role: "user"` for tool results (some implementations require this; if so, document the constraint)
