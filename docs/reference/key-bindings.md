# Key Bindings Reference

All keyboard shortcuts in the hackpi TUI.

The TUI uses an explicit **focus model** — exactly one region or overlay is the
active context at a time. Footer hints are generated dynamically from the key
binding table filtered by the current context. Press `?` at any time to open the
contextual help overlay.

## Global (always available)

| Key | Action |
|-----|--------|
| `Ctrl+C` | Interrupt current generation |
| `Ctrl+L` | Clear conversation |
| `Ctrl+D` | Exit hackpi |
| `?` | Show contextual help overlay |
| `Tab` | Cycle views (Conversation → Task Board → Graph → Conversation) |

## Context-specific bindings

### Composer (text input, resting state)

| Key | Action |
|-----|--------|
| `Enter` | Submit message |
| `Shift+Enter` | Insert newline in input |
| `Esc` | Clear input |
| `/` | Start slash command (opens autocomplete) |

### Conversation scrollback (generating or manual scroll)

| Key | Action |
|-----|--------|
| `Up` / `Down` | Scroll conversation |
| `PgUp` / `PgDn` | Scroll faster |
| `Home` | Scroll to top |
| `End` | Scroll to bottom |

When the app is generating, the scrollback is automatically pinned to the
latest content. Pressing any scroll key switches to manual scroll mode.
Scrolling to the bottom re-enables auto-scroll.

### Task board

| Key | Action |
|-----|--------|
| `Up` / `Down` | Navigate tasks |
| `Enter` | View task detail |
| `n` | Create task |
| `Esc` | Go back to conversation |
| `/` | Start slash command |

### Task detail

| Key | Action |
|-----|--------|
| `Up` / `Down` | Navigate fields |
| `Esc` | Go back to task board |

## Overlay bindings

These take effect when the corresponding overlay is active and trap all keyboard
input until dismissed.

### Help overlay (press `?`)

| Key | Action |
|-----|--------|
| `Esc` | Close help |

### Slash command autocomplete

| Key | Action |
|-----|--------|
| `Up` / `Down` | Navigate commands |
| `Tab` | Select highlighted command |
| `Enter` | Submit selected command |
| `Esc` | Close palette without selecting |

### Permission prompt

| Key | Decision | Risk tier | Persists? |
|-----|----------|-----------|-----------|
| `1` | Allow once | This request | No |
| `3` | Deny | This request | No |
| `2` | Allow until exit | This session | Until exit |
| `4` | Always allow this pattern | Persistent rule | Yes — press `4` twice to confirm |
| `5` | Always deny this pattern | Persistent rule | Yes (saved to config) |
| `Esc` | Cancel | — | No (resets confirmation mode) |

Decisions are grouped by risk tier in the modal. Persistent rules require a
two-step confirmation: press `4` once to enter confirmation mode, then press `4`
again to confirm. Any other key press cancels the confirmation.

### Task creation prompt

| Key | Action |
|-----|--------|
| `Enter` | Create task with entered title |
| `Esc` | Cancel task creation |

## Focus model

The current focus target is determined by the active view and app state:

| View | State | Focus target |
|------|-------|--------------|
| Conversation | Resting | Composer (input field) |
| Conversation | Generating/loading | Conversation scrollback |
| Task board | Any | Task list |
| Task detail | Any | Task detail fields |
| Task graph | Any | Falls back to Composer input |

When an overlay is active (help, permission prompt, autocomplete, task creation),
it traps all keyboard input regardless of the underlying focus target.

## Status bar

The status bar shows dynamically generated footer hints based on the current
context. Key bindings with `footer: true` in the binding table appear as
`[Key] Action` hints. The right side of the status bar shows the connection
health indicator:

| Indicator | Meaning |
|-----------|---------|
| `API: unknown` | No request has been made yet |
| `API: connected` | Last interaction succeeded |
| `API: error` | Last interaction produced an error |
| `API: offline` | Endpoint is unreachable |

Status messages (Generating…, Running bash…, Loading tasks…) appear in the
left/center of the status bar alongside the footer hints.
