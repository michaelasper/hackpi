# [API Design] - [MEDIUM] - Tool schema parameter naming inconsistency: `filePath` vs `path`

**Labels:** `api-design`, `priority-medium`, `consistency`

## Description

The write tool schema uses `filePath` as its path parameter name, while the read, edit, and bash tools all use `path` or `command`/`workdir`. This inconsistency creates confusion for the LLM, which must remember different parameter names for different tools. All path-related tool inputs should follow a consistent naming convention.

Additionally, the write tool schema's `filePath` parameter description says "absolute or relative path" but uses the `filePath` name which differs from the `path` parameter used by read (spec) and edit tools.

## Location

- `hackpi-tools/src/write.rs:32` — `"filePath"`
- `hackpi-tools/src/read.rs:49` — `"path"`
- `hackpi-tools/src/edit/tool.rs:38` — `"path"`
- `write-tool.spec.md:20` — Spec also uses `"filePath"`

## Impact

- LLM may confuse parameter names and fail tool calls
- Inconsistent API surface area makes the agent harder to use and maintain
- The LLM wastes tokens correcting parameter names on retries

## Proposed Solutions

1. Rename `filePath` to `path` in write tool, matching read and edit tools
2. Update the spec file to match
3. Add `"path"` as an alias if backward compatibility with existing models is a concern

## Proposed Solutions

1. Rename `filePath` → `path` in `write.rs` input_schema and `execute` parameter extraction
2. Build a `"path"` → `Message` response correctly
