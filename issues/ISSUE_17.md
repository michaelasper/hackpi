# [Code Review] - [LOW] - Duplicated hash logic between read.rs and edit/hash.rs

**Labels:** `tech-debt`, `priority-low`, `code-review`

## Description

The `line_hash` function and `HASH_CHARS` constant are defined identically in two places:
- `hackpi-tools/src/read.rs:8-19` — standalone `line_hash` function
- `hackpi-tools/src/edit/hash.rs:3-16` — `pub(crate) fn line_hash`

Both implementations use the exact same algorithm (xxHash32, trimmed input, alphanumeric seed logic, same alphabet). This violates DRY and creates a maintenance risk: if the hashing algorithm changes (e.g., different seed logic, different alphabet), one copy may be updated while the other is not, causing silent hash mismatches.

The TODO-05-16.md correctly flags this as C1 ("Hash mismatch on non-alphanumeric lines"), noting that `read.rs` uses `line.as_bytes()` while `edit.rs` uses `trimmed.as_bytes()` for the seed computation. This is a concrete example of why duplicated logic is dangerous.

## Location

- `hackpi-tools/src/read.rs:8-19` — First definition of `line_hash` and `HASH_CHARS`
- `hackpi-tools/src/edit/hash.rs:3-16` — Second definition of `line_hash` and `HASH_CHARS`

## Impact

- Hash mismatches between read output and edit anchor resolution (documented as C1 in TODO)
- Any future hash algorithm change must be made in two places
- Risk of subtle bugs if the two implementations diverge

## Resolution

- Removed duplicate `line_hash` and `HASH_CHARS` from `read.rs`
- `read.rs` now imports `line_hash` from `crate::edit::hash::line_hash`
- Single source of truth for the hashing algorithm — no risk of divergence
- Both functions are in `hackpi-tools`, so `pub(crate)` access works cleanly

**Status: RESOLVED**
