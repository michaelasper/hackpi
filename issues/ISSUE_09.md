# [Dependency Review] - [LOW] - Unused `thiserror` dependency in hackpi-tools

**Labels:** `dependency`, `priority-low`, `tech-debt`

## Description

`hackpi-tools/Cargo.toml` lists `thiserror` as a dependency, but the crate never defines any custom error types. All error handling uses `anyhow` (re-exported from hackpi-core) or string-based `ToolResult::SystemError`. The `thiserror` crate is dragged into the dependency tree unnecessarily.

## Location

- `hackpi-tools/Cargo.toml:11` — `thiserror.workspace = true`

## Impact

- Unnecessary dependency increases compile time
- One more crate in the dependency tree to audit for vulnerabilities
- Inconsistent: hackpi-core also has `thiserror` but doesn't use it either

## Proposed Solutions

1. Remove `thiserror` from both `hackpi-tools/Cargo.toml` and `hackpi-core/Cargo.toml` since neither defines custom error types
2. If `ToolResult` is considered an error type, derive `thiserror::Error` on it and use it consistently across the codebase

## Resolution

**RESOLVED** — Removed `thiserror` from:
- Workspace `Cargo.toml` (workspace dependencies)
- `hackpi-core/Cargo.toml`
- `hackpi-tools/Cargo.toml`
