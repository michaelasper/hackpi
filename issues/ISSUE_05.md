# [Security] - [MEDIUM] - InMemoryFs path traversal can escape above root

**Labels:** `security`, `priority-medium`, `path-traversal`

## Description

In `filesystem.rs`, both `resolve_path_mut` and `resolve_path_ref` handle `..` path components by calling `segments.pop()`, which removes the last segment. However, there is no guard against popping the last element when segments is empty (i.e., when already at root). If a path like `/../../../etc/passwd` is provided, the `/` component is skipped (line 137: `"/" | "." => continue`), then `..` repeatedly calls `segments.pop()` on an empty vector, which is a no-op for Vec. This means `/../../../etc/passwd` resolves to `etc/passwd`, which would traverse up from root — but since root is `/`, this should still stay within root. However, it means that `..` handling is inconsistent: under `create_dir`, `..` is explicitly skipped entirely with `continue`, meaning `mkdir ../foo` creates a sibling directory at the current level instead of one level up.

In `InMemoryFs::write`, `remove`, and `remove_dir`, path components with `".."` are skipped entirely rather than resolved to a parent traversal. This means `echo hello > ../foo.txt` creates `foo.txt` in the current directory instead of the parent.

## Location

- `hackpi-tools/src/bash/filesystem.rs:138-140` — `..` pops segments without boundary check
- `hackpi-tools/src/bash/filesystem.rs:157-159` — Same in `resolve_path_ref`
- `hackpi-tools/src/bash/filesystem.rs:211-215` — `write` skips `..` entirely
- `hackpi-tools/src/bash/filesystem.rs:361-365` — `create_dir` skips `..`
- `hackpi-tools/src/bash/filesystem.rs:273-278` — `remove` skips `..`

## Impact

- Inconsistent behavior: some functions resolve `..` (pop), others skip it entirely
- Commands that use `..` in paths behave unpredictably
- Creating directories with `..` doesn't traverse upward as expected

## Resolution

- Fixed `write()`, `create_dir()`, and `remove()` to handle `..` by popping segments instead of skipping
- `write()`: changed `".." => continue` to pop with boundary check
- `create_dir()`: refactored to use segments Vec with `..` popping
- `remove()`: refactored to use segments Vec with `..` popping
- Added 3 tests: `test_mkdir_with_parent_traversal`, `test_echo_with_parent_traversal`, `test_rm_with_parent_traversal`

**Status: RESOLVED**
