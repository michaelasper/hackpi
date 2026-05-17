# Guardrails Reference

Full documentation of hackpi's permission and guardrail system.

## Config file locations

hackpi loads rules from three files, in order:

| File | Scope |
|------|-------|
| `.hackpi/guardrails.json` | Project-wide rules |
| `.claude/settings.json` | Project-wide rules (shared with Claude) |
| `.claude/settings.local.json` | Local-only rules (not committed) |

Later files override earlier ones. All files use the same JSON format.

## Rule schema

```json
{
  "permissions": {
    "allow": ["Read(./src/**)", "Bash(cargo check)"],
    "deny": ["Write(./.env)", "Bash(curl *)"]
  }
}
```

Each entry is `Tool(pattern)` where:

- `Tool` is one of `Read`, `Write`, `Bash`
- `pattern` is a glob for path-based rules, or a command pattern for bash rules
- Bash patterns use `*` as a wildcard (e.g. `Bash(curl *)` matches any curl command)

## Rule format

| Entry | Tool | Matches |
|-------|------|---------|
| `Read(./src/**)` | read, search_grep | All files under `src/` |
| `Write(./.env)` | write, edit | The `.env` file |
| `Bash(cargo *)` | bash | Any cargo subcommand |
| `Bash(sudo *)` | bash | Any sudo command |
| `Write(./secrets/**)` | write, edit | All files under `secrets/` |

## Evaluation order

1. If `--god` mode is active, all checks return `Allow`
2. Config rules are loaded and merged from all three config files
3. For a tool call with a `command` parameter, `command_gate` checks first
4. For a tool call with a `path` parameter, `file_protection` then `path_guard` check
5. If any guard returns `Deny`, the tool call is blocked
6. If any guard returns `Ask`, the user is prompted in the TUI

## Guard components

### Command gate

Checks bash commands against deny patterns. Blocks dangerous commands like `sudo`, `rm -rf /`, `curl` by default.

### File protection

Checks read/write paths against allow/deny globs. Prevents writing to `.env`, secrets directories, and other protected paths.

### Path guard

Validates that file paths resolve within the workspace root. Blocks path traversal attacks like `../../../etc/passwd`.

## Guard result types

| Result | Meaning |
|--------|---------|
| `Allow` | The tool call proceeds |
| `Deny(reason)` | The tool call is blocked |
| `Ask(reason)` | The user is prompted for a decision |

## Decision persistence

| Decision | Effect | Persisted |
|----------|--------|-----------|
| Allow once | Permitted this one time | No |
| Allow for session | Permitted until hackpi exits | No (in memory) |
| Deny | Blocked this one time | No |
| Always allow | Permitted forever | Yes — `.claude/settings.local.json` |
| Always deny | Blocked forever | Yes — `.claude/settings.local.json` |

## Hot reload

Config files are watched for changes. When you edit a guardrails JSON file:

1. The new file is parsed
2. If valid, rules are swapped in immediately
3. If invalid, the old rules are kept and a warning is logged