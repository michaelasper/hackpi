# [Architecture] - [LOW] - `workspace_root` parameter accepted but unused in bash `with_session`

**Labels:** `tech-debt`, `priority-low`, `architecture`

## Description

In `session.rs:348`, the `with_session` function signature accepts `workspace_root: &PathBuf` but immediately discards it with `let _ = workspace_root;`. The function was clearly designed to use the workspace root for filesystem operations (e.g., mounting an OverlayFs or ReadWriteFs backed by the real workspace), but the implementation only uses `InMemoryFs` and ignores the parameter entirely.

This means bash operations are always confined to the virtual in-memory filesystem and cannot read or write real files on disk. While this is a valid safety choice, the function signature is misleading and the parameter adds noise without purpose.

## Location

- `hackpi-tools/src/bash/session.rs:348` — `let _ = workspace_root;`
- `hackpi-tools/src/bash/tool.rs:76-78` — `workspace_root` cloned and passed but never used
- `hackpi-tools/src/bash/session.rs:323-325` — Function signature accepts `workspace_root`

## Impact

- Misleading API: callers pass workspace_root expecting it to matter
- Prevents future use of OverlayFs (read real files, write to memory) without code changes
- The spec describes OverlayFs and ReadWriteFs as planned implementations but they're stubbed

## Proposed Solutions

1. Remove the `workspace_root` parameter from `with_session` entirely
2. Or implement `OverlayFs` backed by the workspace root so bash can read real files
3. Or add a `BashConfig` struct that specifies which filesystem backend to use

## Resolution

**Status:** RESOLVED

**Changes made:**
- Removed `workspace_root: &PathBuf` parameter from `with_session` in `hackpi-tools/src/bash/session.rs:323`
- Removed `let _ = workspace_root;` discard line from `with_session` body
- Updated call site in `hackpi-tools/src/bash/tool.rs:77` — removed `&wr` argument and the `let wr = self.workspace_root.clone();` line
- Added test `test_with_session_works_without_workspace_root` in `hackpi-tools/src/bash/tests.rs` that calls `with_session` without the removed parameter
