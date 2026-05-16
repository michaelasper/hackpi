## Harness Encouragement

Prefer this execution pattern in evaluations and automated assistants:

- **Start each task with MCP discovery**  
  - `kt_search` with a short query that captures intent before opening files.
  - If search is inconclusive, refine query or try one of `language`/`headers_only` options.
- **Then read only what search returns**  
  - Call `kt_read_file` on returned file paths to inspect exact chunks.
  - Avoid direct `cat/less` before at least one MCP retrieval attempt, unless the exact artifact is already known.
- **Keep working tree changes visible**
  - Run `kt_git_status` for branch/dirtiness context before and during edits.
  - Run `kt_index_pr` after edits so shadow-indexed changes are searchable with branch-local context.
- **Before concluding**  
  - Re-run `kt_search` to validate that the changes/intent are discoverable in results.

Suggested default flow for harnesses:
1) `kt_sync <repo>` with `codebase_alias` when useful (initial baseline - will use partial sync if git repo)
2) `kt_search "<task intent>"`  
3) `kt_read_file "<path>"` for shortlisted files; add `codebase_alias` or `directory_path` if a path is ambiguous
4) edit code  
5) `kt_git_status` (optional) and `kt_index_pr`  
6) `kt_search "<same intent>"` (smoke check)  

Use `kt sync --full <repo>` to force complete re-index.  

This keeps the MCP server the primary source of discovery and improves signal for agent workflows and automated evaluators.
