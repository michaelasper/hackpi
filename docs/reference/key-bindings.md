# Key Bindings Reference

All keyboard shortcuts in the hackpi TUI.

## Global

| Key | Action |
|-----|--------|
| `Enter` | Submit input |
| `Shift+Enter` | Insert newline in input |
| `Ctrl+C` | Interrupt current generation |
| `Ctrl+L` | Clear conversation |
| `Ctrl+D` | Exit hackpi |

## Navigation

| Key | Action |
|-----|--------|
| `Esc` | Dismiss autocomplete popover / Cancel task creation / Go back to previous view (TaskDetail → TaskBoard, TaskBoard/TaskGraph → Conversation) |
| `Tab` | Cycle views: Conversation → TaskBoard → TaskGraph → Conversation |

## Scrolling

| Key | Action |
|-----|--------|
| `PgUp` | Scroll conversation up |
| `PgDn` | Scroll conversation down |
| `Home` | Scroll to top |
| `End` | Scroll to bottom |

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