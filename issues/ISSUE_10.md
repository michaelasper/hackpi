# [Architecture] - [MEDIUM] - No slash command handler despite `/help` being advertised in UI

**Labels:** `missing-feature`, `priority-medium`, `architecture`

## Description

The status bar advertises `/help` as an available command (`Ctrl+C interrupt  Ctrl+L clear  Ctrl+D exit  /help`), and the TUI spec lists four slash commands (`/help`, `/clear`, `/model`, `/ctx`, `/quit`). However, there is no slash command parser or handler anywhere in the codebase. When a user types `/help` and presses Enter, the message is sent directly to the LLM as a regular user message. The LLM receives it as a code query and tries to process it as such.

Additionally, `Ctrl+L` calls `app.clear()` (clearing the conversation), and `Ctrl+D` exits — both of which work. But the advertised `/help` command does nothing special, and the `/clear`, `/model`, `/ctx`, `/quit` slash commands are entirely unimplemented.

## Location

- `hackpi-tui/src/main.rs:186-187` — Status bar text advertises `/help`
- `hackpi-tui/src/app.rs` — No slash command handling
- `hackpi-tui/src/main.rs:134-165` — Key handling loop doesn't intercept `/` as a slash command trigger

## Impact

- Users typing `/help` get confusing LLM responses instead of a help screen
- Misleading UI: advertising a feature that doesn't exist
- Spec compliance gap: TUI spec section "Slash Commands" lists 5 commands, none implemented

## Proposed Solutions

1. Add a slash command parser in `main.rs` that intercepts messages starting with `/` before sending to the LLM
2. Implement at minimum `/help` (display a help popup or inline text) and `/clear` (same as Ctrl+L)
3. Remove `/help` from the status bar if slash commands are deferred
