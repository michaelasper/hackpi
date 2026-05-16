# [Bugs] - [MEDIUM] - `Anyhow` and `thiserror` imported but never used in hackpi-core

**Labels:** `tech-debt`, `priority-low`, `dependency`

## Description

`hackpi-core/Cargo.toml` lists both `anyhow` and `thiserror` as dependencies, but:
- `hackpi-core/src/api.rs:2` uses `anyhow::Result` for the `send_messages` return type
- `hackpi-core/src/agent.rs` does NOT use anyhow (it constructs error strings manually)
- `thiserror` is never used anywhere in hackpi-core
- `hackpi-core/src/tools.rs` defines `ToolResult` as a manual enum without `thiserror`

The `anyhow` dependency is underutilized: `send_messages` returns `anyhow::Result<()>`, but errors are converted to strings via `format!("API error: {e}")` at the call site in `agent.rs:75`, losing the structured error chain that `anyhow` provides.

## Location

- `hackpi-core/Cargo.toml:12-13` — `anyhow` and `thiserror` listed
- `hackpi-core/src/api.rs:2` — `use anyhow::Result`
- `hackpi-core/src/api.rs:57` — `?` operator propagates errors via anyhow
- `hackpi-core/src/agent.rs:75` — Error converted to string, losing anyhow context

## Impact

- `anyhow` is only used in one function signature and immediately stringified at the call site
- `thiserror` is entirely unused
- The error chain is lost at the agent boundary; the model receives a flat string with no details
- Two unnecessary dependencies increase compile time

## Proposed Solutions

1. Either use `anyhow` consistently (attach context with `.context()`) or remove it and use `Box<dyn Error>`
2. Remove `thiserror` from both crate Cargo.tomls since it's never used
3. Pass richer error information through `AgentEvent::Error` so the TUI can display it usefully
