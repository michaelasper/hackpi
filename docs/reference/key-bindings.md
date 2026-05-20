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
| `?` | Show context help overlay |

## Focus-specific bindings

| Context | Key | Action |
|---------|-----|--------|
| **Composer** | `Enter` | Submit message |
| | `Esc` | Clear input |
| | `Tab` | Cycle views |
| **Conversation** | `Up` / `Down` | Scroll |
| | `PgUp` / `PgDn` | Scroll faster |
| | `Home` | Scroll to top |
| | `End` | Scroll to bottom |
| **Task board** | `Up` / `Down` | Navigate tasks |
| | `Enter` | View task detail |
| | `n` | Create task |
| | `Esc` | Go back to conversation |
| **Task detail** | `Up` / `Down` | Navigate fields |
| | `Esc` | Go back to task board |

## Overlay bindings

These take effect when the corresponding overlay is active:

| Overlay | Key | Action |
|---------|-----|--------|
| **Help overlay** | `Esc` | Close help |
| **Autocomplete** | `Up` / `Down` | Navigate commands |
| | `Tab` | Select command |
| | `Enter` | Submit command |
| | `Esc` | Close palette |
| **Permission prompt** | `1`–`5` | Choose decision |
| | `Esc` | Deny |
| **Task creation** | `Enter` | Create task |
| | `Esc` | Cancel |

## Focus model

The current focus target is determined by the active view and app state:

| View | State | Focus target |
|------|-------|--------------|
| Conversation | Resting | Composer (input field) |
| Conversation | Generating | Scrollback |
| Task board | Any | Task list |
| Task detail | Any | Task detail |

When an overlay is active (help, permission prompt, autocomplete, task creation),
it traps all keyboard input until dismissed.

## Permission prompts

When a permission prompt is active, number keys select decisions:

| Key | Decision | Persists? |
|-----|----------|-----------|
| `1` | Allow once | No |
| `2` | Allow for session | Until exit |
| `3` | Deny | No |
| `4` | Always allow | Yes (saved to config) |
| `5` | Always deny | Yes (saved to config) |
| `Esc` | Deny (same as `3`) | No |
