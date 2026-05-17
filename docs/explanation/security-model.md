# Security Model

hackpi's security model is built around three principles: sandboxed execution, strict boundaries, and explicit user consent.

## Sandboxed execution

The `bash` tool does not shell out to a real process. Instead, it implements commands in-process with a virtual filesystem:

- **No arbitrary binary execution.** Only built-in commands are available. There is no `./script.sh`, no `python script.py`, no executing downloaded files.
- **No network access by default.** `curl`, `wget`, and similar commands are not registered. Network access requires explicit configuration with URL allow-lists.
- **Execution limits.** Configurable limits prevent infinite loops (max 10 000 iterations), runaway command chains (max 10 000 commands), and deep recursion (max 50 call depth).

These constraints apply automatically. The agent cannot bypass them.

## Strict boundaries

### Write tool jail

The `write` tool rejects any path that resolves outside the workspace root. This prevents the agent from writing to `~/.bashrc`, `/etc/hosts`, or any file outside the project directory.

### Path guard

The `path_guard` component validates that every file path in read, write, and edit operations resolves within the workspace. Symlink traversal and `../` sequences are resolved before checking.

### New-file-only contract

The `write` tool will not overwrite existing files. It hard-fails with an error message directing the agent to use `edit` instead. This prevents the agent from clobbering existing files with a full-file rewrite, which is simpler for the model but destructive in practice.

### Atomic writes

All file writes use a temp-file-then-rename strategy. If the process crashes mid-write, the original file is untouched. Symlink chains are resolved so the target file is updated without replacing the symlink. Hard-linked files are updated in place to preserve the shared inode.

## Explicit user consent

### Guardrails

The `hackpi-guardrails` crate provides three guard components:

1. **Command gate** — checks bash commands against allow/deny patterns
2. **File protection** — checks file paths against read/write allow/deny globs
3. **Path guard** — validates paths stay within the workspace root

When a tool call matches no explicit allow or deny rule, the user is prompted with options: allow once, allow for the session, deny, always allow, or always deny. "Always" decisions persist to `.claude/settings.local.json`.

### God mode

For trusted environments, `hackpi --god` bypasses all guard checks. This is intended for sandboxes, CI, and other scenarios where the agent has full trust. Use with caution.

## Why these choices matter

Coding agents operate on your code. Traditional agents with unrestricted `bash` access can modify anything on your system. hackpi's layered approach means:

- Even if the LLM hallucinates a destructive command, the virtual filesystem and command registry prevent it from executing
- Even if the model tries to write outside the workspace, the path jail blocks it
- Even if an action is technically allowed, guardrails give you a chance to review and deny it

The goal is not to make the agent helpless, but to ensure every action is scoped, bounded, and auditable.