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

## Proposed Solutions

1. Use `grep-searcher`'s built-in `Searcher::search_paths()` with a path iterator instead of pre-collecting
2. Stream paths through the walk and search each one immediately
3. If pre-collection is intentional (e.g., for sorting/max-matches cutoff), add a comment explaining why
