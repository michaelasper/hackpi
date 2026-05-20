<p align="center">
  <b>hackpi</b>
</p>

<p align="center">
  <i>A local-first coding agent with hash-anchored edits, sandboxed execution, and a full terminal UI.</i>
</p>

<p align="center">
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/rust-2021-orange?logo=rust" alt="Rust 2021"></a>
  <a href="https://crates.io/"><img src="https://img.shields.io/badge/version-0.1.0-blue" alt="v0.1.0"></a>
</p>

---

## TL;DR

**Problem:** Coding agents that edit files silently relocate changes, run arbitrary shell commands, and require cloud APIs.

**Solution:** hackpi is a Rust coding agent that anchors every edit to a hash of the line being replaced, runs bash in a virtual filesystem with no network access, and streams everything through a terminal UI built on ratatui.

| Feature | Benefit |
|---------|---------|
| Hash-anchored edits | Refuses stale anchors — no silent relocations |
| Virtual bash filesystem | Sandboxed execution, no arbitrary exec |
| Context-aware ripgrep | Search with `context_lines` built in |
| 256 KB result cap + overflow to temp files | Prevents context blowup on long outputs |
| Streaming TUI | Watch the agent think and act in real time |
| Local-first API client | Works with DeepSeek V4 Flash on localhost |
| **Task board** | Create, view, and transition tasks across workflow states |
| **Guardrails** | Configurable allow/deny rules, permission prompts, session caching |
| **Git & GitHub** | View git status, log, and list PRs directly from the TUI |
| **Conversation export** | Export full conversation history to a text file |

## Quick Start

```bash
# 1. Install (requires Rust)
curl -sSL https://raw.githubusercontent.com/michaelasper/hackpi/main/install.sh | bash

# 2. Set your endpoint
export HACKPI_ENDPOINT=http://localhost:11434/api/chat
export HACKPI_MODEL=llama3.2

# 3. Run
hackpi
```

Or build from source:

```bash
git clone https://github.com/michaelasper/hackpi.git
cd hackpi
cargo build --release -p hackpi-tui
cp target/release/hackpi /usr/local/bin/
```

## Commands

hackpi is a TUI application — launch it and type natural-language requests.

### Global keys (always available)

| Key | Action |
|-----|--------|
| `Ctrl+C` | Interrupt current generation |
| `Ctrl+L` | Clear conversation |
| `Ctrl+D` | Exit hackpi |
| `?` | Show contextual help overlay |
| `Tab` | Cycle views (Conversation → Tasks → Graph → Conversation) |

### Context-specific keys

| Context | Key | Action |
|---------|-----|--------|
| **Composer** (input) | `Enter` | Submit message |
| | `Shift+Enter` | Insert newline |
| | `/` | Start slash command (opens autocomplete) |
| **Conversation** (scrollback) | `Up` / `Down` | Scroll conversation |
| | `PgUp` / `PgDn` | Scroll faster |
| | `Home` | Scroll to top |
| | `End` | Scroll to bottom |
| **Task board** | `Up` / `Down` | Navigate tasks |
| | `Enter` | View task detail |
| | `n` | Create task |
| | `Esc` | Go back to conversation |
| **Task detail** | `Up` / `Down` | Navigate fields |
| | `Esc` | Go back to task board |

### Slash Commands

Type `/` in the input to open the autocomplete popover, then type to filter:

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/clear` | Clear the conversation |
| `/quit` | Exit the application |
| `/guardrails:status` | Show guardrails status |
| `/guardrails:clean` | Clear session cache |
| `/guardrails:onboarding [preset]` | Write a preset guardrails config (strict, balanced, permissive) |
| `/git:status` | Show git status |
| `/git:log` | Show recent git log |
| `/github:pr-list` | List open pull requests |
| `/task create <title>` | Create a new task |
| `/task list` | List all tasks |
| `/task show <id>` | Show task details |
| `/task move <id> <state>` | Move task to a new state |
| `/task done <id>` | Mark task as done |
| `/task block <id> <blocked_by>` | Add blocking dependency |
| `/task unblock <id> <blocked_by>` | Remove blocking dependency |
| `/task label <id> <label>` | Add a label to a task |
| `/task assign <id> <assignee>` | Assign task to someone |
| `/tasks` | Alias for `/task list` |
| `/export [path]` | Export conversation to text file |

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HACKPI_ENDPOINT` | `http://localhost:11434/api/chat` | LLM API endpoint |
| `HACKPI_MODEL` | `llama3.2` | Model name |
| `HACKPI_MAX_TOKENS` | `4096` | Maximum tokens per response |

## Architecture

```
┌──────────┐     ┌──────────────┐     ┌───────────┐
│  TUI     │────▶│  Agent Loop  │────▶│  API      │
│  Events  │     │ (hackpi-core) │     │  Client   │
└──────────┘     └──────┬───────┘     └───────────┘
                        │
                   ┌────▼───────┐
                   │   Tool     │
                   │  Registry  │
                   └────┬───────┘
                        │
              ┌─────────┼─────────┐
              │         │         │
         ┌────▼───┐ ┌──▼───┐ ┌───▼────┐
         │ bash   │ │ edit │ │ read   │
         │(tools) │ │(tools)│ │(tools) │
         └────────┘ └──────┘ └────────┘
```

### Crates

| Crate | Purpose |
|-------|---------|
| `hackpi-core` | Agent loop, API client, tool registry, shared types |
| `hackpi-tools` | `read`, `search_grep`, `edit`, `write`, `bash` |
| `hackpi-tui` | ratatui terminal interface, event channels, input handling |
| `hackpi-guardrails` | Path validation, file protection, command gating with permission prompt system |
| `hackpi-tasks` | Task store, workflow state machine, slash-command task management |
| `hackpi-vcs` | Git read operations and GitHub PR listing via slash commands |

### Tool system

| Tool | Description |
|------|-------------|
| `read` | Read files with hash-anchored line numbers |
| `search_grep` | Context-aware ripgrep wrapper with `context_lines` |
| `edit` | Replace/append/prepend lines anchored to hashes — rejects stale anchors |
| `write` | Atomic file creation jailed to workspace root |
| `bash` | Virtual shell with in-memory filesystem, built-in commands, pipes, and redirects |
| `git_read` | Read-only git operations: status, diff, log, branches, remotes |
| `git_write` | Mutating git operations: add, commit, push, merge, branch, rebase, stash |
| `github` | GitHub operations: PRs, issues, labels, and releases |
| `task` | Task management with workflow-defined states and blocking dependencies |
| `bash` | Virtual filesystem with command registry — no network, no arbitrary exec |
| `git_read` | Read git status and log |
| `github` | List GitHub pull requests |
| `task` | Create, list, show, move, and manage tasks |

### Design decisions

- **Hash-anchored edits**: Every `read` output includes a hash per line. Edits reference those hashes. If the file changed since the read, the edit is rejected rather than silently relocated.
- **Streaming tool results**: Tool output streams back to the LLM in the same turn. No batched delivery.
- **Turn limit**: Hard cap at 25 tool-use rounds per request. After that, the agent returns what it has.
- **Deterministic**: `temperature=0`, flat tool schemas, minimal system prompt overhead.

## Installation

### Quick install (recommended)

```bash
curl -sSL https://raw.githubusercontent.com/michaelasper/hackpi/main/install.sh | bash
```

### Cargo install

```bash
cargo install --git https://github.com/michaelasper/hackpi hackpi-tui
```

### Build from source

```bash
git clone https://github.com/michaelasper/hackpi.git
cd hackpi
cargo build --release -p hackpi-tui
cp target/release/hackpi /usr/local/bin/
```

### Custom install location

```bash
INSTALL_DIR="$HOME/.local/bin" ./install.sh
```

## Troubleshooting

### Error: "Rust is not installed"

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Error: "Permission denied" during install

```bash
sudo INSTALL_DIR="/usr/local/bin" ./install.sh
# Or use a user-writable directory:
INSTALL_DIR="$HOME/.local/bin" ./install.sh
```

### Error: "Binary not found in PATH"

```bash
export PATH="$PATH:/usr/local/bin"
# Add to ~/.bashrc or ~/.zshrc for persistence
```

## Limitations

| Limitation | Detail | Workaround |
|------------|--------|------------|
| Local LLM only | Optimized for DeepSeek V4 Flash on localhost | Other Anthropic-format APIs may work |
| No arbitrary shell exec | Bash tool uses a virtual filesystem | Use `write` + external terminal for untrusted commands |
| 256 KB per tool result | Large outputs truncated to temp file | Re-read the temp file with `read` |
| 25-turn cap | Agent stops after 25 rounds | Continue in a new request |

## Documentation

### Tutorials (learn by doing)
- [Getting Started](docs/tutorials/getting-started.md) — Install hackpi, connect to an LLM, make your first edit

### How-to Guides (solve a problem)
- [Configure Guardrails](docs/how-to/configure-guardrails.md) — Set up allow/deny rules, respond to permission prompts
- [Edit Files with Hash Anchors](docs/how-to/edit-files.md) — Read, edit, and chain operations
- [Connect to Different LLM Providers](docs/how-to/connect-llm.md) — Point hackpi at different API endpoints

### Reference (look things up)
- [Tools Reference](docs/reference/tools.md) — Full schemas and parameters for all five tools
- [Key Bindings](docs/reference/key-bindings.md) — TUI keyboard shortcuts
- [Guardrails Reference](docs/reference/guardrails.md) — Rule format, evaluation order, persistence
- [Environment Variables](docs/reference/environment-variables.md) — Configuration variables

### Explanation (understand concepts)
- [Why Hash Anchors?](docs/explanation/hash-anchors.md) — The motivation and design behind hash-anchored editing
- [Security Model](docs/explanation/security-model.md) — Sandboxed execution, path jails, and guardrails
- [Architecture](docs/explanation/architecture.md) — Crate structure, data flow, and streaming tool results

### Specification files

Detailed implementation specs for each subsystem:

- [hashline.spec.md](hashline.spec.md) — Edit system, LINE#HASH anchoring, diff preview
- [tui.spec.md](tui.spec.md) — TUI layout, key bindings, event channels
- [read-tool.spec.md](read-tool.spec.md) — `read` and `search_grep` tools
- [write-tool.spec.md](write-tool.spec.md) — `write_file` tool, atomic writes, path jail
- [bash-tool.spec.md](bash-tool.spec.md) — Virtual bash, filesystem trait, command registry

## Contributing

Issues and pull requests are welcome at [github.com/michaelasper/hackpi](https://github.com/michaelasper/hackpi).