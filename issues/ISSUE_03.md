# [Performance] - [MEDIUM] - Single Mutex serializes all InMemoryFs operations

**Labels:** `performance`, `priority-medium`, `architecture`

## Description

`InMemoryFs` wraps its entire filesystem state (`FileNode` tree) in a single `std::sync::Mutex`. Every filesystem operation — read, write, read_dir, metadata, exists — acquires this lock. Since the bash session is single-threaded in practice (commands execute sequentially), this is acceptable for the current architecture. However, the filesystem implementation uses `resolve_path_mut` (mutable borrow) even for read-only operations like `read()` and `exists()`, needlessly taking a write lock via the Mutex.

In `filesystem.rs:180`, `read()` calls `self.root.lock().unwrap()` then immediately calls `resolve_path_mut`, which requires mutable access to traverse children. This means even pure reads block all other would-be readers.

## Location

- `hackpi-tools/src/bash/filesystem.rs:51-53` — `root: Mutex<FileNode>`
- `hackpi-tools/src/bash/filesystem.rs:179-196` — `read()` acquires mutex and uses `resolve_path_mut`
- `hackpi-tools/src/bash/filesystem.rs:312-315` — `exists()` acquires mutex
- `hackpi-tools/src/bash/filesystem.rs:331-351` — `read_dir()` acquires mutex

## Impact

- Concurrent read/write operations are serialized, though this is not currently a bottleneck since bash is single-threaded
- Using `resolve_path_mut` for read-only operations is semantically incorrect and may mask bugs if the tree is mutated unexpectedly
- Prevents future parallel filesystem access optimizations

## Resolution

- Changed `Mutex<FileNode>` to `RwLock<FileNode>` in `InMemoryFs`
- Read-only operations (`read`, `exists`, `is_dir`, `is_file`, `read_dir`, `metadata`) use `read().unwrap()`
- Write operations (`write`, `remove`, `create_dir`) use `write().unwrap()`
- `read()` changed from `resolve_path_mut` to `resolve_path_ref` (read-only)
- Removed unused `resolve_path_mut` function entirely
- Added `test_concurrent_reads_do_not_deadlock` test with 10 concurrent readers

**Status: RESOLVED**
