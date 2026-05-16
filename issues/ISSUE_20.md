# [Security] - [LOW] - Hard-coded temp file paths with model-controlled tool IDs create injection risk

**Labels:** `security`, `priority-low`

## Description

In `agent.rs:186-188`, truncated tool output is written to a temp file with a path built from the tool ID:
```rust
let tmp_path = self.workspace_root.join(format!(".truncated_{tool_id}.txt"));
```

The `tool_id` (e.g., `"toolu_..."`) comes from the LLM's `content_block_start` event. While the LLM response is trusted in the current architecture, the tool_id value is not sanitized before being used in a file path. The API spec allows tool IDs to contain arbitrary ASCII characters. If a malicious or misconfigured server sent a tool_id like `../../etc/cronjob`, the path would resolve outside the workspace.

This is a low-severity issue because:
- The LLM server is assumed to be local and trusted (`127.0.0.1`)
- The workspace_root jail in the write tool would catch most escapes
- The temp file path is only used for the reference message to the LLM

However, it's a defense-in-depth violation that should be addressed.

## Location

- `hackpi-core/src/agent.rs:187` — Path constructed from untrusted `tool_id`

## Impact

- Potential path traversal if the API server is compromised or misconfigured
- Temp files with `.txt` extension may leak into unexpected directories

## Proposed Solutions

1. Sanitize `tool_id` by stripping all non-alphanumeric characters before using it in a path
2. Use a counter-based temp filename instead of embedding the tool_id
3. Verify the constructed path is within `workspace_root` before writing
