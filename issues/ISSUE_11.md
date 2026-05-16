# [Performance] - [MEDIUM] - search_grep collects all file paths before searching

**Labels:** `performance`, `priority-medium`

## Description

In `search_grep.rs:99-114`, the `walkdir` function first collects ALL matching file paths into a `Vec<PathBuf>`, then iterates over the vector to search each file with `grep-searcher`. For large repositories with thousands of files, this means:
1. Walking the entire directory tree twice (once in walkdir's `filter_entry`, once in walkdir's inner loop)
2. Allocating a potentially large Vec of PathBufs upfront
3. Holding all paths in memory before any searching begins

The `grep-searcher` crate natively supports walking directories and filtering, which would stream results without pre-collecting.

## Location

- `hackpi-tools/src/search_grep.rs:99-114` — Pre-collection of all file paths
- `hackpi-tools/src/search_grep.rs:223-249` — `walkdir` function builds full Vec

## Impact

- Memory proportional to repo file count (every matched file path is allocated)
- Wasted latency: search doesn't begin until the full walk is complete
- For 100k+ file repos, this could be a noticeable delay

## Resolution (Deferred)

This optimization changes the search architecture significantly by replacing the two-pass approach (walk → collect → search) with a single-pass streaming approach. The fix would require rewriting the `walkdir` integration in `search_grep.rs` to search files as they're discovered rather than pre-collecting into a `Vec<PathBuf>`.

**Deferred to post-v1** — current implementation is correct and performant for repos under ~10k files. The pre-collection enables the `MAX_MATCHES` cutoff across files, which would need a different mechanism in streaming mode.

## Implementation Notes (for future work)

### 1. The ignore Crate Integration

Since you are already using `grep-searcher` (part of the ripgrep core ecosystem), pair it with the `ignore` crate (also by BurntSushi). It natively handles recursive directory walking, respects `.gitignore`, and is built specifically to pipe discovered paths efficiently into a searcher.

### 2. Managing the MAX_MATCHES Cutoff

How you handle the cutoff in a streaming model depends on your concurrency:

- **Single-threaded:** Keep a mutable running tally of matches. `walkdir` provides an iterator; process files as `walkdir` yields them and explicitly `break` the loop when the tally hits `MAX_MATCHES`.

- **Multi-threaded:** Use an `Arc<AtomicUsize>` passed to search workers to track total global matches. Workers can check `matches.load(Ordering::Relaxed)` and bail out early if the limit is reached.

### 3. Latency vs. Throughput

By streaming, you drastically reduce the Time-To-First-Match (TTFM). In massive repos, users care more about seeing the first 10 results instantly than waiting 3 seconds to see all 1,000. Streaming optimizes for that UX.

**Status: BACKLOGGED**
