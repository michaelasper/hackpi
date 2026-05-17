# Tools Reference

This page documents every tool available in hackpi, including its schema, parameters, output format, and edge cases.

## read

Read a file or directory. Returns content with `LINE#HASH:` prefixes for use with the edit tool.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `path` | string | yes | — | File or directory path to read |
| `offset` | integer | no | 1 | Start reading from this line (1-indexed) |
| `limit` | integer | no | all | Maximum number of lines to return |

### Output format

Text files return with `LINE#HASH:` prefixes on each line:

```
 8#VR:function hello() {
 9#KT:  console.log("world");
10#BH:}
```

### Content type handling

| Type | Behaviour |
|------|-----------|
| Text file | Return with `LINE#HASH:` prefixes |
| Directory | List entries: `dir/  src/`, `file  Cargo.toml` |
| Image (PNG/JPEG/GIF/WebP) | Pass through as attachment, no hashline |
| Binary file | Reject with descriptive error |
| Empty file | Return advisory suggesting `prepend`/`append` |

### Large file handling

Files over 1000 lines are truncated to the first 200 lines with a summary:

```
... [truncated: 1842 total lines, showing 200] ...
```

Use `offset` and `limit` to read specific sections.

### Hash algorithm

xxHash32 via the `xxhash-rust` crate, mapped to 2 characters from alphabet `ZPMQVRWSNKTXJBYH`. Lines with no alphanumeric characters use their line number as the hash seed.

---

## search_grep

Search the codebase for a regex pattern with surrounding context lines.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `pattern` | string | yes | — | Regular expression to search for |
| `include_glob` | string | no | — | Glob pattern to restrict search (e.g. `src/**/*.rs`) |
| `context_lines` | integer | no | 2 | Lines before and after each match (max 10) |

### Output format

```
src/auth.rs:42:  pub fn handle_auth(token: &str) -> Result<User> {
src/auth.rs:44:      let decoded = decode_token(token)?;
src/auth.rs:47:      Ok(user)
---
src/db.rs:12:  use crate::auth::AuthStrategy;
```

Match blocks separated by `---`. Each line includes file path, line number, and content.

### Limits

| Limit | Value |
|-------|-------|
| Maximum matches | 50 |
| Line length cap | 500 characters |
| Context lines max | 10 |

If over 50 matches, output includes:

```
[Search truncated. Over 50 matches found. Refine your pattern or use include_glob.]
```

Lines over 500 characters are replaced with:

```
[line omitted: 1243 characters — exceeds 500 char limit]
```

### Filtering

- `.gitignore` is respected
- Hard-ignores: `node_modules/`, `target/`, `.git/`, `.terraform/`, `dist/`, `build/`
- Binary files are skipped
- Hidden files are skipped by default
- `include_glob` is applied via the `globset` crate

---

## edit

Modify existing files using `LINE#HASH` anchors from read output.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `path` | string | yes | File path to edit |
| `edits` | array | yes | List of edit operations |

### Operations

| Op | Purpose | Fields |
|----|---------|--------|
| `replace` | Replace one line or an inclusive range | `pos` required, `end` optional, `lines` required |
| `append` | Insert lines after `pos` (or at EOF if omitted) | `pos` optional, `lines` required |
| `prepend` | Insert lines before `pos` (or at BOF if omitted) | `pos` optional, `lines` required |
| `replace_text` | Replace an exact unique substring | `oldText` required, `newText` required |

### Example

```json
{
  "path": "src/main.rs",
  "edits": [
    { "op": "replace", "pos": "9#KT", "lines": ["  console.log(\"hackpi\");"] }
  ]
}
```

### Chained edits

Multiple edits in one call validate against the same pre-edit snapshot and apply bottom-up. After success, the result includes an `--- Updated anchors ---` block with fresh hashes for changed lines.

### Error conditions

| Error | Condition |
|-------|-----------|
| Stale anchor | Hash does not match current file content |
| `[E_INVALID_PATCH]` | Lines contain `LINE#HASH:` prefixes or diff `+`/`-` markers |

Stale anchor errors include a snippet with fresh hashes for immediate retry.

---

## write

Create a new file. **Hard-fails if the file already exists** — use `edit` to modify existing files.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `filePath` | string | yes | Path for the new file. Parent directories are created automatically. |
| `content` | string | yes | Complete text content of the file |

### Safety

- Path jail: writes must stay within the workspace root
- Atomic writes: content is buffered, written to a temp file, then renamed
- Parent directories are created automatically

### Error conditions

| Error | Condition |
|-------|-----------|
| File exists | Target file already exists — use `edit` instead |
| Path escape | Path resolves outside workspace root |
| Permission denied | Insufficient filesystem permissions |

---

## bash

Execute a command in a persistent virtual shell with an in-memory filesystem.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `command` | string | yes | — | Bash command to execute |
| `timeout` | integer | no | 30 | Timeout in seconds (max 120) |
| `workdir` | string | no | cwd | Working directory override |

### Supported shell syntax

| Feature | Example |
|---------|---------|
| Simple commands | `ls -la` |
| Pipes | `cat foo \| grep bar` |
| Output redirect | `echo hello > file.txt` |
| Append redirect | `echo more >> file.txt` |
| Input redirect | `cat < input.txt` |
| Stderr redirect | `cmd 2> err.log` |
| AND/OR chaining | `cmd1 && cmd2` \| `cmd1 \|\| cmd2` |
| Sequential | `cmd1; cmd2` |
| Variables | `echo $HOME`, `NAME=value cmd` |
| Quoting | `'single'`, `"double $var"`, `\"escaped\"` |
| Subshell | `$(echo hi)` |
| Comments | `# comment` |

### Built-in commands

Navigation: `cd`, `pwd`, `echo`, `env`, `export`, `ls`

File ops: `cat`, `cp`, `mv`, `rm`, `mkdir`, `touch`, `ln`

Text processing: `grep`, `head`, `tail`, `sort`, `wc`, `cut`, `tr`, `uniq`

### Security

- No network access by default (no `curl`, `wget`, etc.)
- No arbitrary binary execution
- Path traversal protection on all filesystem operations
- Execution limits: max 50 call depth, 10000 commands, 10000 loop iterations