# How to Configure Guardrails

This guide shows you how to set up permission guardrails that control what the agent can do in your workspace.

## Prerequisites

- hackpi installed (see [Getting Started](../tutorials/getting-started.md))
- A project directory with a `.hackpi/` or `.claude/` configuration directory

## Create a guardrails config

Create `.hackpi/guardrails.json` in your project root:

```json
{
  "permissions": {
    "allow": [
      "Read(./src/**)",
      "Bash(cargo check)",
      "Bash(cargo test)"
    ],
    "deny": [
      "Write(./.env)",
      "Bash(curl *)",
      "Bash(sudo *)"
    ]
  }
}
```

Each rule is a `Tool(path_or_command)` pattern:

- `Read(./src/**)` — allow reading any file under `src/`
- `Write(./.env)` — deny writing to `.env`
- `Bash(curl *)` — deny any `curl` command
- `Bash(sudo *)` — deny any `sudo` command

## Use god mode to skip all guards

If you trust the agent and want to skip permission prompts:

```bash
hackpi --god
```

All guard checks return `Allow` in god mode. Use this in sandboxed environments only.

## Respond to permission prompts in the TUI

When the agent tries an action that matches no allow or deny rule, you see a permission prompt:

```
┌─ Permission Request ──────────────────────────┐
│ Tool: bash                                    │
│ Reason: CommandGate: "rm -rf" matches a       │
│         dangerous pattern                     │
│                                               │
│ [1] Allow once    [2] Allow for session       │
│ [3] Deny          [4] Always allow            │
│ [5] Always deny                                │
└───────────────────────────────────────────────┘
```

Press the number key that matches your choice:

| Key | Decision | Effect |
|-----|----------|--------|
| `1` | Allow once | Permitted this one time |
| `2` | Allow for session | Permitted until you exit hackpi |
| `3` | Deny | Blocked this time |
| `4` | Always allow | Permitted forever, saved to config |
| `5` | Always deny | Blocked forever, saved to config |

Choices `4` and `5` persist to `.claude/settings.local.json` so they survive restarts.

## Hot-reload config changes

hackpi watches your config files. When you edit `.hackpi/guardrails.json` or `.claude/settings.json`, the rules are reloaded automatically — no restart required.

If the new config has a syntax error, hackpi keeps the old rules and logs a warning.

## Common patterns

### Allow read access to everything, deny writes to secrets

```json
{
  "permissions": {
    "allow": ["Read(./**)", "Bash(cargo *)"],
    "deny": ["Write(./.env)", "Write(./**/*secret*)"]
  }
}
```

### Allow all bash commands except network access

```json
{
  "permissions": {
    "allow": ["Bash(*)"],
    "deny": ["Bash(curl *)", "Bash(wget *)", "Bash(ssh *)"]
  }
}
```

## Further reading

- [Guardrails reference](../reference/guardrails.md) for full rule schema and evaluation order
- [Security model explanation](../explanation/security-model.md) for how guardrails fit into hackpi's security architecture