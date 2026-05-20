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

---

## git_read

Read-only git operations for inspecting repository state.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `operation` | string | yes | One of: `status`, `diff`, `diff_staged`, `log`, `branch_list`, `remote_list`, `show` |
| `count` | integer | no | Number of log entries (default: 20, max: 100) |
| `revision` | string | no | Revision for `show` (HEAD, branch name, or hash) |

### Operations

| Operation | Output |
|-----------|--------|
| `status` | Working tree status (staged, unstaged, untracked changes) |
| `diff` | Unstaged diff of working tree changes |
| `diff_staged` | Diff of staged (index) changes |
| `log` | Recent commit history with hashes, authors, dates, and messages |
| `branch_list` | List of local branches with current branch marker |
| `remote_list` | List of configured remotes with URLs |
| `show` | Full diff for a specific commit or reference |

### Environment

- Uses the repository containing the current working directory
- No authentication required (read-only)

---

## git_write

Mutating git operations with safety checks.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `operation` | string | yes | One of: `add`, `commit`, `push`, `pull`, `fetch`, `checkout`, `branch_create`, `branch_delete`, `merge`, `rebase`, `stash`, `stash_pop`, `reset` |
| `paths` | array of strings | no | File paths for `add` or `checkout` operations |
| `all` | boolean | no | Stage all changes (for `add`) |
| `message` | string | no | Commit message or stash message |
| `remote` | string | no | Remote name (default: `origin`) |
| `branch` | string | no | Branch name for checkout/create/delete/merge |
| `force` | boolean | no | Force push |
| `create` | boolean | no | Create branch when checking out |

### Destructive operations

Some operations modify git history or remote state:

| Operation | Safety note |
|-----------|-------------|
| `push` | Requires `--force` flag for force push |
| `reset` | Defaults to `--mixed` (keeps working tree changes) |
| `merge` | Creates merge commits by default (no fast-forward) |
| `rebase` | Rewrites local branch history |
| `branch_delete` | Deletes a local branch |

### Environment

- Uses the repository containing the current working directory
- Uses the git user config for commit authorship

---

## github

GitHub operations for PRs, issues, labels, and releases.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `operation` | string | yes | One of: `pr_create`, `pr_list`, `pr_merge`, `pr_checkout`, `issue_create`, `issue_list`, `issue_close`, `issue_comment`, `label_add`, `label_list`, `release_create`, `release_list` |
| `owner` | string | no | Repository owner (inferred from git remote if omitted) |
| `repo` | string | no | Repository name (inferred from git remote if omitted) |
| `title` | string | no | Title for PR or issue creation |
| `head` | string | no | Head branch for PR |
| `base` | string | no | Base branch for PR (e.g. `main`) |
| `body` | string | no | Body text for PR, issue, comment, or release |
| `draft` | boolean | no | Create PR or release as draft |
| `number` | integer | no | PR/issue number for merge/checkout/close/comment |

### Authentication

Requires `HACKPI_GITHUB_TOKEN` environment variable (falls back to `GITHUB_TOKEN`).

### Destructive operations

| Operation | Safety note |
|-----------|-------------|
| `pr_merge` | Merges a pull request |
| `pr_checkout` | Fetches and checks out a PR branch locally |
| `issue_close` | Closes an issue |
| `release_create` | Creates a GitHub release |

---

## task

Manage tasks with workflow-defined state transitions.

### Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `operation` | string | yes | One of: `create`, `list`, `show`, `update`, `transition`, `block`, `unblock` |
| `title` | string | no | Task title (required for `create`) |
| `description` | string | no | Task description |
| `id` | string | no | Task ID in `TSK-XXX` format |
| `state` | string | no | Target state for `transition` |
| `priority` | string | no | One of: `none`, `low`, `medium`, `high`, `urgent` |
| `labels` | array of strings | no | Labels for the task |
| `assignee` | string | no | Assignee identifier |
| `blocked_by` | string | no | Task ID to block this task |

### Operations

| Operation | Description |
|-----------|-------------|
| `create` | Create a new task with title, description, priority, labels |
| `list` | List all tasks with state, priority, blocking info |
| `show` | Show full task details by ID |
| `update` | Update task fields (title, description, priority, labels, assignee) |
| `transition` | Move task to a new state (validated against workflow) |
| `block` | Add a blocking dependency on another task |
| `unblock` | Remove a blocking dependency |

### Workflow states

Tasks follow a configurable state machine. The default workflow is:
`Backlog → Todo → In Progress → In Review → Staged/Ready → Done`

Transitions are validated — invalid transitions return a clear error.