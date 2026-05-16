# Read & Search Tool Spec (v1)

Two coordinated tools for codebase exploration: `search_grep` for finding code by pattern, and `read` for inspecting full files with hashline anchors.

## search_grep — Context-Aware Ripgrep Wrapper

The gold-standard search tool for agentic workflows. Wraps ripgrep in-process for zero-overhead search with native context awareness.

### Tool Schema (Anthropic Format)

```json
{
  "name": "search_grep",
  "description": "Searches the codebase for a regex pattern. Returns matching lines with surrounding context.",
  "input_schema": {
    "type": "object",
    "properties": {
      "pattern": {
        "type": "string",
        "description": "The regular expression to search for."
      },
      "include_glob": {
        "type": "string",
        "description": "Optional glob pattern to restrict the search (e.g. 'src/**/*.rs')."
      },
      "context_lines": {
        "type": "integer",
        "description": "Number of lines to include before and after each match. Max 10. Default 2."
      }
    },
    "required": ["pattern"]
  }
}
```

### Native Context Trick

The `context_lines` parameter is the key innovation. By returning surrounding lines alongside each match, the agent can understand code context without issuing a separate `read` call. Benchmarks show this reduces follow-up file reads by ~40% and cuts wall-clock time by ~20%.

The model infers the tradeoff naturally: small context (0-2 lines) for broad searches, larger context (5-10 lines) when it needs to understand the function body around a match.

### Output Format

```
src/auth.rs:42:  pub fn handle_auth(token: &str) -> Result<User> {
src/auth.rs:44:      let decoded = decode_token(token)?;
src/auth.rs:47:      Ok(user)
---
src/db.rs:12:  use crate::auth::AuthStrategy;
```

Each match block is separated by `---`. Each line includes file path, line number, and the line content (with context lines).

### Hard Safety Limits

1. **Gitignore-aware**: Natively respects `.gitignore`. Hard-ignores `node_modules/`, `target/`, `.git/`, `.terraform/`, `dist/`, `build/`.

2. **Match cap**: Maximum 50 matches returned. If the search hits the limit, append to output:
   ```
   [Search truncated. Over 50 matches found. Refine your pattern or use include_glob.]
   ```

3. **Line length cap**: Lines over 500 characters are omitted and replaced with:
   ```
   [line omitted: 1243 characters — exceeds 500 char limit]
   ```

4. **Binary files**: Automatically skipped (ripgrep's default behavior).

### Implementation

- **Engine**: `grep-searcher` + `grep-regex` crates (in-process ripgrep, no fork/exec)
- **Parallel search**: Uses `grep-searcher`'s built-in parallel search (multi-threaded)
- **Searcher config**:
  - `.gitignore` respect: enabled
  - Hidden files: skipped by default
  - Binary detection: enabled
  - Encoding: auto-detect (UTF-8, UTF-16)
- **Path filtering**: `include_glob` applied via globset crate

---

## read — Hashline File Reader

Reads files and returns content with `LINE#HASH:` prefixes for use with the hashline edit system.

### Tool Schema (Anthropic Format)

```json
{
  "name": "read",
  "description": "Read a file or directory. Returns file contents with LINE#HASH: prefixes for editing.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Path to the file or directory to read."
      },
      "offset": {
        "type": "integer",
        "description": "Start reading from this line number (1-indexed). Default: 1."
      },
      "limit": {
        "type": "integer",
        "description": "Maximum number of lines to return. Default: all lines."
      }
    },
    "required": ["path"]
  }
}
```

### Output Format

```
 8#VR:function hello() {
 9#KT:  console.log("world");
10#BH:}
```

Each line prefixed with `LINE#HASH:`, where:
- `LINE`: 1-indexed line number, left-padded for alignment
- `HASH`: 2-character xxHash32 digest from alphabet `ZPMQVRWSNKTXJBYH`

### Behavior by Content Type

| Type | Behavior |
|---|---|
| Text file | Return with LINE#HASH: prefixes |
| Directory | List entries: `dir/  src/`, `file  Cargo.toml` |
| Image (PNG/JPEG/GIF/WebP) | Pass through as attachment, no hashline |
| Binary file | Reject with descriptive error |
| Empty file | Return advisory suggesting `prepend`/`append` |

### Large File Handling

Files over 1000 lines are truncated:
- Return first 200 lines with `... [truncated: 1842 total lines, showing 200] ...`
- Include summary: file size, line count, detected language
- Model can use `offset`/`limit` to read specific sections

### Hashline Algorithm

- **Crate**: `xxhash-rust` (xxHash32)
- **Alphabet**: `ZPMQVRWSNKTXJBYH` (16 chars, excludes hex digits, vowels, ambiguous letters)
- **Mapping**: xxHash32 output `u32` → 2 chars via modulo 16
- **Edge case**: Lines with no alphanumeric characters use line number as hash seed to reduce collisions

### Relationship to search_grep

- `search_grep` finds code by pattern with context — first step in understanding
- `read` provides full file content with hashline anchors — for editing or deep inspection
- The `context_lines` parameter on `search_grep` eliminates most follow-up `read` calls
- When editing is needed, call `read` with `offset`/`limit` on the matching region to get hashed lines
