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

## Resolution

- Added `handle_slash_command()` function in `app.rs` that handles `/help`, `/clear`, `/quit`
- `/help` sends help text as `StreamChunk` events followed by `Done`
- `/clear` calls `app.clear()`
- `/quit` sets `app.quit_requested = true`
- Unknown commands send an `Error` event
- 4 tests added for slash command handling

**Status: RESOLVED**
