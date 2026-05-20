# Self-Testing Harness for HackPI

**COR-268** — Research on using hackpi to test hackpi itself

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│                  HackPI TUI                      │
│                                                   │
│  ┌──────────┐   key events    ┌──────────────┐   │
│  │crossterm │ ──────────────→ │  Event Loop   │   │
│  │  stdin   │                 │  (main.rs)    │   │
│  └──────────┘                 │               │   │
│                               │  ┌─────────┐  │   │
│  ┌──────────┐   render call   │  │ App.rs  │  │   │
│  │ ratatui  │ ←────────────── │  │ (state) │  │   │
│  │ terminal │                 │  └─────────┘  │   │
│  └──────────┘                 │       │       │   │
│                               │  ┌─────────┐  │   │
│                               │  │ ui.rs   │  │   │
│                               │  │(render) │  │   │
│                               │  └─────────┘  │   │
│                               └───────────────┘   │
└─────────────────────────────────────────────────┘
```

## Key Finding: Ratatui TestBackend Already Available

The codebase **already uses** `ratatui::backend::TestBackend` for unit testing the render functions. Example from `ui.rs`:

```rust
let backend = TestBackend::new(80, 24);
let mut terminal = ratatui::Terminal::new(backend).unwrap();
let mut app = App::new();
terminal.draw(|f| render(f, &app)).unwrap();
let buffer = terminal.backend().buffer();
// Assert on buffer.content...
```

This means **snapshot testing of rendered output** is already supported.

## What's Missing for Full Integration Testing

### Gap 1: No headless/scripted mode
The event loop in `main.rs` reads from crossterm `event::read()` which requires a real TTY. There's no `--headless` flag that accepts pre-recorded input events.

### Gap 2: No synthetic event injection
The App state can be constructed manually, but there's no way to feed synthetic `Event::Key(...)` events through the full input→app→render pipeline without the real event loop.

### Gap 3: No structured output mode
There's no `--json` or `--structured` mode that outputs events (rendered text, tool results, errors) as structured data instead of rendering to a terminal.

## Recommended Approach: Two-Phase Implementation

### Phase 1: Scripted Test Mode (minimal, high-value)

Add a `--script <file>` mode to hackpi that:
1. Reads a JSON file containing a sequence of input events + optional assertions
2. Creates the App state, event handlers, and renderer in memory
3. Processes each event through the App
4. After each event, renders to a TestBackend and checks assertions
5. Outputs pass/fail for each test step

**Test script format (JSON):**
```json
{
  "name": "Clear command clears conversation",
  "steps": [
    {
      "action": "submit", 
      "text": "Hello",
      "assert": {
        "conversation_len": 1,
        "render_contains": "Hello"
      }
    },
    {
      "action": "submit",
      "text": "/clear",
      "assert": {
        "conversation_len": 1,
        "render_contains": "cleared"
      }
    }
  ]
}
```

**Why this works:** The `App` struct already handles events via `app.handle_event(event)`, and the render function takes `&App`. We can create a test harness that:
- Creates an `App` + `TestBackend` terminal
- Constructs `TuiEvent` variants directly (no crossterm needed)
- Calls `app.handle_event(event)` 
- Calls `terminal.draw(|f| render(f, &app))`
- Asserts on the buffer contents

### Phase 2: Self-Testing via /export + compare

A meta-approach where hackpi:
1. Runs a real session (by the developer or LLM agent)
2. Exports via `/export`
3. The exported file becomes the "expected" output
4. A re-run of the same inputs produces a new export
5. `diff expected.txt actual.txt` validates the behavior

### Phase 3: CI Integration
Wire the scripted test runner into CI (GitHub Actions) so every PR runs the automated TUI test suite.

## Implementation Plan (Follow-up Issues)

### COR-269: Add `--script` mode to hackpi TUI (Phase 1 core)
- Parse `--script <path>` argument in main()
- Create a test harness that reads JSON scenario files
- Process events through App state without crossterm
- Render to TestBackend after each step
- Assert on conversation state and rendered output
- Exit with appropriate status code

### COR-270: Write scripted test scenarios for existing TUI behavior
- /help command renders correctly
- /clear clears conversation
- /quit requests exit
- /export saves file
- Tab cycling through views
- Task creation flow
- Permission prompt flow
- Ctrl+C interrupt works
- Ctrl+L clear works
- Autocomplete filtering and selection
- Input cursor positioning
- Ghost textbox prevention
- Guardrails slash commands

### COR-271: Add structured event output mode
- `--structured-events` flag that outputs machine-readable JSON events
- Each TuiEvent becomes a JSON line on stdout
- Enables external tools to monitor hackpi behavior
- Useful for debugging and for test harness assertions

### COR-272: Wire test scenarios into CI
- Add GitHub Actions workflow step
- Run `hackpi --script scenarios/*.json`
- Report pass/fail per scenario
- Block PR merge on failures
