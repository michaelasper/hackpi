# Getting Started with hackpi

In this tutorial we will install hackpi, connect it to a local LLM, and make our first code edit. By the end you will have hackpi running against a live model and understand the edit workflow.

## Prerequisites

- Rust 1.70 or later (install with `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- A running LLM server that exposes an Anthropic-compatible `/v1/messages` endpoint (such as ds4-server with DeepSeek V4 Flash)

## Step 1 — Install hackpi

Run the install script:

```bash
curl -sSL https://raw.githubusercontent.com/michaelasper/hackpi/main/install.sh | bash
```

Verify the installation:

```bash
hackpi --help
```

You should see hackpi's version output. If you get "command not found", add the install directory to your PATH:

```bash
export PATH="$PATH:/usr/local/bin"
```

## Step 2 — Configure your endpoint

hackpi connects to a local LLM. Set the endpoint environment variable:

```bash
export HACKPI_ENDPOINT=http://localhost:8080/v1/messages
export HACKPI_MODEL=deepseek-v4-flash
export HACKPI_MAX_TOKENS=4096
```

Add these lines to your shell profile (`~/.bashrc` or `~/.zshrc`) so they persist.

If you are using an Ollama server instead, you will need a proxy that translates
the Ollama `/api/chat` format into the Anthropic `/v1/messages` format hackpi
expects (e.g., [litellm](https://github.com/BerriAI/litellm) with
`--model ollama/llama3.2 --port 8080`).

## Step 3 — Launch hackpi

Open a terminal in your project directory and start hackpi:

```bash
cd my-project
hackpi
```

You will see the TUI: a header bar at the top, a conversation area, and an input bar at the bottom.

## Step 4 — Read a file

Type a request and press Enter:

```
> read src/main.rs
```

The agent calls the `read` tool and displays the file with `LINE#HASH:` prefixes on each line:

```
 1#VR:fn main() {
 2#KT:    println!("hello");
 3#BH:}
```

Notice the hash anchors (`VR`, `KT`, `BH`) — you will use these for editing.

## Step 5 — Make an edit

Ask the agent to change the file:

```
> change the print message to "hello hackpi"
```

The agent calls the `edit` tool with a hash anchor:

```json
{ "op": "replace", "pos": "2#KT", "lines": ["    println!(\"hello hackpi\");"] }
```

If the hash matches the current file, the edit is applied. If the file has changed since the read, the edit is rejected with a suggestion to re-read — no silent relocations.

## Step 6 — Create a new file

```
> create a new file src/lib.rs with a pub fn greet() that prints a greeting
```

The agent uses the `write` tool for new files. If the file already exists, `write` is rejected and the agent falls back to `edit`.

## Step 7 — Interrupt and resume

While the agent is generating, press `Ctrl+C` to interrupt. The agent stops and you get your input back. Type a new message to continue.

Press `Ctrl+D` to exit hackpi entirely. Press `Ctrl+L` to clear the conversation and start fresh.

## What you learned

- hackpi reads files with hash anchors and edits them with those anchors
- New files use `write`; existing files use `edit`
- `Ctrl+C` interrupts, `Ctrl+D` exits, `Ctrl+L` clears

Next, read the [how-to guides](../how-to/configure-guardrails.md) for configuring guardrails, or the [reference](../reference/tools.md) for full tool schemas.