# [Architecture] - [MEDIUM] - No `additionalProperties: false` on most tool schemas (except write)

**Labels:** `api-design`, `priority-medium`, `consistency`

## Description

The write tool schema sets `"additionalProperties": false`, which tells the LLM API that no extra parameters are accepted beyond those listed. However, the read, edit, search_grep, and bash tool schemas do NOT set this constraint. This means:

1. The LLM can hallucinate extra parameters and the API won't reject them
2. The LLM wastes tokens generating parameters that are silently ignored
3. Inconsistent schema design across tools

This is especially problematic for the edit tool, where the schema defines `lines`, `newText`, `oldText`, `pos`, `end`, and `op` per edit item, but without `additionalProperties: false`, the LLM could send extra fields per edit that are silently dropped by `deserialize_edit_ops`.

## Location

- `hackpi-tools/src/write.rs:45` — Has `"additionalProperties": false`
- `hackpi-tools/src/read.rs:46-63` — Missing `additionalProperties`
- `hackpi-tools/src/edit/tool.rs:34-80` — Missing `additionalProperties`
- `hackpi-tools/src/search_grep.rs:34-53` — Missing `additionalProperties`
- `hackpi-tools/src/bash/tool.rs:28-47` — Missing `additionalProperties`

## Impact

- LLM may generate invalid extra parameters
- Schema validation is inconsistent across tools
- The edit tool's inner schema (per-edit-item) also lacks `additionalProperties`, meaning the LLM could send `lines` AND `newText` on the same edit, and one would be silently ignored

## Proposed Solutions

1. Add `"additionalProperties": false` to all top-level tool schemas
2. Add `"additionalProperties": false` to the edit item sub-schema in `edit/tool.rs`
3. Consider using serde's `deny_unknown_fields` on deserialization structs as a defense-in-depth measure
