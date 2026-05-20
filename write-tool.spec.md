# Write Tool Spec

Creates new files atomically with workspace boundary enforcement. Existing files are handled exclusively by the hashline edit tool — `write_file` hard-fails if the target already exists.

## Design Rationale

- **Single-purpose, flat schema**: Keeps JSON output predictable regardless of inference engine (frontier API or local MLX). Multi-command schemas degrade on local models.
- **New-file-only contract**: Agents will try to overwrite existing files because edit payloads are harder. Brutally reject this — it teaches the boundary immediately without crashing the loop.
- **Atomic writes**: Full content is buffered then renamed into place. No partial writes, no corrupted files on crash.

## Tool Schema (Anthropic Format)

```json
{
  "name": "write",
  "description": "Creates a completely new file at the specified path with the provided content. CRITICAL: This tool will hard-fail if the file already exists. To modify existing files, you MUST use the edit tool instead.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "The absolute or relative path where the new file should be created (e.g., 'src/agent/orchestrator.rs'). Parent directories will be created automatically if they do not exist."
      },
      "content": {
        "type": "string",
        "description": "The complete, raw text content to write to the new file. Do not wrap in markdown code blocks unless the file itself requires it."
      }
    },
    "required": ["path", "content"],
    "additionalProperties": false
  }
}
```

## Rust Harness

### The Overwrite Trap

```rust
let file_path = workspace_root.join(&args.path);

if file_path.exists() {
    return Ok(ToolResponse::SystemError(
        "Error: File already exists at this path. You cannot overwrite files with write. \
         You must use the edit tool to modify existing code.".to_string()
    ));
}
```

Return the error directly to the model's context window — teaches the boundary without crashing the loop.

### The Phantom Directory Handler

Models often assume folder structures exist when scaffolding new modules.

```rust
if let Some(parent) = file_path.parent() {
    tokio::fs::create_dir_all(parent).await.map_err(|e| ...)?;
}
```

### Path Jail (Workspace Boundary)

Prevents the agent from hallucinating paths or escaping to `~/.bashrc`, `/etc/hosts`, etc.

```rust
let canonical = file_path.canonicalize().unwrap_or(file_path);
if !canonical.starts_with(&workspace_root) {
    return Ok(ToolResponse::SystemError(
        "Security Error: Attempted to write outside the workspace directory.".to_string()
    ));
}
```

### Atomic Write

Write to a temp file in the same directory, then rename. Ensures no partial writes survive crashes.

```rust
let tmp_path = parent.join(format!(".{}.tmp", file_name));
tokio::fs::write(&tmp_path, &args.content).await?;
tokio::fs::rename(&tmp_path, &file_path).await?;
```

### Error Classification

All errors returned to the model should be instructive — the model sees the error in its context window and self-corrects on the next turn.

| Error | Message |
|---|---|
| File exists | `"Error: File already exists. Use edit to modify."` |
| Path escape | `"Security Error: Attempted to write outside workspace."` |
| Permission denied | `"Permission denied: [path]"` |
| Disk full / IO | `"IO error: [detail]"` |

### Memory Footprint

Large `content` payloads can blow up context window size. Strategies:

1. **Pass content by value in the tool call struct** — let the LLM framework handle serialization.
2. **After execution, clamp the stored content** in the conversation history for display purposes (e.g., truncate to first/last 100 lines, or store a hash + line count).
3. **For extremely large files (>10k lines)**, stream the write but store only a metadata stub (`{path, size, hash, lines}`) in the conversation history rather than the full content.

Default behavior in v1: keep full content in history (simple, correct). Add content clamping as a performance optimization in v1.1 if context window pressure becomes an issue.
