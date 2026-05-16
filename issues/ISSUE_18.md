# [Bugs] - [MEDIUM] - Edit tool uses sync `std::fs::write/rename` while write tool uses async `tokio::fs`

**Labels:** `bug`, `priority-medium`, `consistency`

## Description

The write tool (`write.rs:102,112`) uses `tokio::fs::write` and `tokio::fs::rename` (async, non-blocking), while the edit tool (`tool.rs:376,386`) uses `std::fs::write` and `std::fs::rename` (sync, blocking). Both are called from async `execute` methods in the tool trait.

The edit tool's synchronous file I/O will block the tokio runtime thread. While tokio's default multi-threaded runtime can tolerate brief blocking, the `std::fs::write` call for large files could block the thread for a measurable amount of time. The write tool (which handles the same file-writing pattern) got the async treatment, but the edit tool (which also writes files) was left with sync I/O.

This was already noted in TODO-05-16.md item M15 but only for the write tool — the edit tool has the same issue but was not flagged.

## Location

- `hackpi-tools/src/edit/tool.rs:376` — `std::fs::write(&tmp_path, result.as_bytes())`
- `hackpi-tools/src/edit/tool.rs:386` — `std::fs::rename(&tmp_path, &canonical)`
- `hackpi-tools/src/edit/tool.rs:382-383` — `std::fs::set_permissions`

## Impact

- Blocks the async runtime during file writes
- For very large files being edited, this could cause noticeable UI stutter
- Inconsistent with write tool's async approach

## Proposed Solutions

1. Replace `std::fs::write` with `tokio::fs::write` in `edit/tool.rs:376`
2. Replace `std::fs::rename` with `tokio::fs::rename` in `edit/tool.rs:386`
3. Replace `std::fs::set_permissions` with `tokio::fs::set_permissions`
4. Wrap the sync operations in `tokio::task::spawn_blocking` if async variants are insufficient
