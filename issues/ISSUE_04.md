# [Architecture] - [MEDIUM] - Edit tool resolves anchors against pre-edit snapshot but diff preview uses stale old_snippet

**Labels:** `bug`, `priority-medium`, `architecture`, `edit-system`

## Description

In `edit/tool.rs`, edit operations are resolved against the original `lines` (pre-edit snapshot) but applied to `current_lines` (accumulating edits). The `AppliedEdit.old_snippet` is captured from `lines[start_lineno..end_idx]` (the original), while the replacement is made to `current_lines`. Since edits are applied bottom-up, the line numbers in `lines` (original) may not correspond to the same content in `current_lines` (after prior edits), but the `old_snippet` is always from `lines`.

For a single edit, this is correct. But for multiple edits that overlap or are adjacent, the `old_snippet` shown in the diff preview may not reflect the actual content that was present at that location when the edit was applied, because prior edits may have already modified those lines in `current_lines`.

Additionally, for `Append` and `Prepend` with `pos`, the `old_snippet` is computed as `current_lines[insert_at..insert_at]` (an empty slice), which means the diff preview never shows what was there before, making the "Diff preview" uninformative for insertions.

## Location

- `hackpi-tools/src/edit/tool.rs:260-364` — Edit application loop
- `hackpi-tools/src/edit/tool.rs:278-279` — `old_snippet` captured from original `lines`
- `hackpi-tools/src/edit/tool.rs:305-309` — Empty `old_snippet` for append operations
- `hackpi-tools/src/edit/tool.rs:330-334` — Empty `old_snippet` for prepend operations

## Impact

- Diff preview can show inaccurate old content for multi-edit operations
- Append/prepend edits show no "before" content in the diff, making the preview less useful
- The LLM cannot reliably verify what changed when multiple edits are applied

## Proposed Solutions

1. Capture `old_snippet` from `current_lines` (the actual state at the time of the edit) instead of `lines` (the original snapshot)
2. For append/prepend, capture the actual adjacent lines (before/after the insertion point) from `current_lines` rather than an empty slice
3. Add a test case for multi-edit diff preview accuracy
