# [Silent Failures] - [HIGH] - Swallowed errors in truncated output temp file write

**Labels:** `bug`, `priority-high`, `silent-failures`

## Description

In `agent.rs:188`, when tool output exceeds the 256KB limit, the full output is written to a temp file. However, the `std::fs::write` call's `Result` is discarded with `let _ =`. If this write fails (disk full, permission denied, path too long), the model receives a truncated message claiming `"Full output written to /path/.truncated_xyz.txt"` — but the file was never actually written. The model has no way to know the reference is invalid.

Additionally, the path is constructed by joining `workspace_root` with `.truncated_{tool_id}.txt`. The tool_id comes from the LLM response and could theoretically contain path traversal characters.

## Location

- `hackpi-core/src/agent.rs:188`

## Impact

- Model receives a misleading "Full output written to..." reference that points to a non-existent file
- If the file write fails, the model's next read attempt will fail with "file not found," wasting a turn
- No logging or fallback when the temp write fails

## Proposed Solutions

1. Check the result of `std::fs::write` and log a warning if it fails
2. If the temp file write fails, append `[Failed to write full output to disk]` to the truncated message instead of the file path
3. Strip path traversal characters from tool_id before using it in a file path
