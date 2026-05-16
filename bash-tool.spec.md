# Bash Tool Spec (v1)

A virtual bash environment with an in-memory filesystem, written in Rust and designed for AI agents. Port of the [just-bash](https://github.com/vercel-labs/just-bash) concept from TypeScript to Rust.

## Core Concept

Instead of shelling out to a real bash process via PTY, implement bash commands in-process with a virtual filesystem layer. This gives:

- **Security**: No arbitrary binary execution, network off by default, URL allow-lists
- **Determinism**: Each command execution is fully controlled and trackable
- **Extensibility**: Custom commands are Rust functions, not forked processes
- **Zero dependencies on system shell**: Works on any OS without bash installed

## Architecture

```
┌─────────────────────────────────────────┐
│           BashSession                    │
│  ┌───────────────────────────────────┐  │
│  │         Shell Parser              │  │
│  │  tokenize → AST → execute        │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │       Command Registry            │  │
│  │  HashMap<String, CommandFn>       │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │        Virtual FileSystem         │  │
│  │  InMemoryFs │ OverlayFs │ RWFs    │  │
│  └───────────────────────────────────┘  │
│  ┌───────────────────────────────────┐  │
│  │      Shell State (env, cwd)       │  │
│  └───────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

## Tool Schema (Anthropic Format)

```json
{
  "name": "bash",
  "description": "Execute a bash command in a persistent virtual shell. The filesystem persists across calls.",
  "input_schema": {
    "type": "object",
    "properties": {
      "command": {
        "type": "string",
        "description": "The bash command to execute."
      },
      "timeout": {
        "type": "integer",
        "description": "Timeout in seconds (default: 30, max: 120)."
      },
      "workdir": {
        "type": "string",
        "description": "Working directory override (absolute path)."
      }
    },
    "required": ["command"]
  }
}
```

## Shell Parser

### Supported Syntax (v1)

| Feature | Example |
|---|---|
| Simple commands | `ls -la` |
| Pipes | `cat foo \| grep bar` |
| Output redirect | `echo hello > file.txt` |
| Append redirect | `echo more >> file.txt` |
| Input redirect | `cat < input.txt` |
| Stderr redirect | `cmd 2> err.log` |
| Stderr-to-stdout | `cmd 2>&1` |
| AND chaining | `cmd1 && cmd2` |
| OR chaining | `cmd1 \|\| cmd2` |
| Sequential | `cmd1; cmd2` |
| Variables | `echo $HOME`, `NAME=value cmd` |
| Quoting | `'single'`, `"double $var"`, `\"escaped\"` |
| Subshell | `$(echo hi)` |
| Comments | `# this is a comment` |

### Parser Implementation

Custom hand-written recursive descent parser:
1. **Tokenizer**: split input into tokens (words, pipes, redirects, operators, quotes)
2. **AST**: build a tree of `Command`, `Pipeline`, `Redirect`, `AndOr`, `Sequence` nodes
3. **Executor**: walk the AST, resolving variables, performing redirects, dispatching commands

The parser does NOT need to be fully bash-compatible. Aim for 90% of real-world usage. Gaps will be filled as discovered.

## Command Registry

Commands are registered as `fn(args: &[String], ctx: &mut CommandContext) -> Result<CommandOutput>`.

```rust
pub struct CommandContext {
    pub fs: &mut dyn FileSystem,
    pub env: &mut HashMap<String, String>,
    pub cwd: &mut PathBuf,
    pub stdin: Option<String>,
    pub stdout: &mut Vec<u8>,
    pub stderr: &mut Vec<u8>,
    pub signal: &Option<tokio::sync::watch::Receiver<bool>>,
}
```

### V1 Commands

#### Navigation & Environment

| Command | Description |
|---|---|
| `cd [path]` | Change directory |
| `pwd` | Print working directory |
| `echo [-n] [args...]` | Print arguments |
| `env` | Print environment variables |
| `export NAME=value` | Set environment variable |
| `ls [-la] [path...]` | List directory contents |

#### File Operations

| Command | Description |
|---|---|
| `cat [files...]` | Concatenate and print files |
| `cp src dst` | Copy files |
| `mv src dst` | Move/rename files |
| `rm [-rf] path` | Remove files |
| `mkdir [-p] path` | Create directories |
| `touch path` | Create/update file timestamp |
| `ln [-s] target link` | Create links |

#### Text Processing

| Command | Description |
|---|---|
| `grep [-i] pattern [files...]` | Search for pattern |
| `head [-n N] [file]` | First N lines |
| `tail [-n N] [file]` | Last N lines |
| `sort [-r] [-n] [file]` | Sort lines |
| `wc [-lwc] [file]` | Line/word/char count |
| `cut -d DELIM -f FIELDS [file]` | Cut columns |
| `tr SET1 SET2` | Translate characters |
| `uniq [-c] [file]` | Unique lines |

All commands support `--help` flag.

### Future Commands (Post-v1)

`sed`, `awk`, `find`, `diff`, `patch`, `tee`, `printf`, `base64`, `od`, `xxd`, `comm`, `join`, `paste`, `fold`, `expand`, `unexpand`, `split`, `nl`, `strings`, `tac`, `rev`, `shuf`, `tsort`, `jq` (JSON), `yq` (YAML), `curl`, `python3`, `js-exec`, `sqlite3`, `tar`, `gzip`, `gunzip`, `zcat`, `md5sum`, `sha256sum`, `time`, `sleep`, `timeout`, `seq`, `which`, `whoami`, `hostname`, `date`, `clear`, `tee`, `xargs`

## Virtual Filesystem

### Filesystem Trait

```rust
pub trait FileSystem: Send {
    fn read(&mut self, path: &Path) -> Result<Vec<u8>>;
    fn write(&mut self, path: &Path, content: &[u8]) -> Result<()>;
    fn append(&mut self, path: &Path, content: &[u8]) -> Result<()>;
    fn remove(&mut self, path: &Path) -> Result<()>;
    fn rename(&mut self, from: &Path, to: &Path) -> Result<()>;
    fn copy(&mut self, from: &Path, to: &Path) -> Result<()>;
    fn exists(&mut self, path: &Path) -> bool;
    fn is_dir(&mut self, path: &Path) -> bool;
    fn is_file(&mut self, path: &Path) -> bool;
    fn read_dir(&mut self, path: &Path) -> Result<Vec<DirEntry>>;
    fn create_dir(&mut self, path: &Path, recursive: bool) -> Result<()>;
    fn remove_dir(&mut self, path: &Path, recursive: bool) -> Result<()>;
    fn metadata(&mut self, path: &Path) -> Result<FileMeta>;
    fn symlink(&mut self, target: &Path, link: &Path) -> Result<()>;
    fn read_link(&mut self, path: &Path) -> Result<PathBuf>;
}
```

### Implementations

**InMemoryFs** (default, always available)

Pure in-memory filesystem. No disk access. Files stored in `BTreeMap<PathBuf, FileNode>`:
```rust
struct FileNode {
    content: Vec<u8>,
    mode: u32,
    is_symlink: bool,
    symlink_target: Option<PathBuf>,
    created: SystemTime,
    modified: SystemTime,
}
```

**OverlayFs** (read from disk, write to memory)

Copy-on-write over a real directory. Reads fall through to disk, writes stay in memory. Useful for giving the agent access to a project without risk of disk modification.

```rust
struct OverlayFs {
    root: PathBuf,           // real disk path
    overlay: InMemoryFs,     // in-memory writes
}
```

**ReadWriteFs** (direct disk access)

Direct read-write to a real directory. Use with caution — pointed at a workspace directory.

```rust
struct ReadWriteFs {
    root: PathBuf,           // real disk path, sandbox root
}
```

Operations are validated to stay within `root`. Path traversal attacks are rejected.

**MountableFs** (combine multiple filesystems)

Mount different filesystems at different paths:

```rust
struct MountableFs {
    base: Box<dyn FileSystem>,
    mounts: Vec<(PathBuf, Box<dyn FileSystem>)>,
}
```

### Default Filesystem Layout (InMemoryFs)

```
/home/user/          # Default cwd and $HOME
/tmp/                # Temporary files
/dev/null            # /dev/null support
```

## Execution Model

1. **Persistent session**: One `BashSession` per tool invocation chain. Filesystem, env, and cwd persist across calls.
2. **Per-command isolation**: Each `exec()` call gets a fresh shell state (env overrides, cwd override) but shares the filesystem.
3. **Timeout**: Configurable per-exec. Hard abort at statement boundary.
4. **Cancellation**: Via `AbortSignal`-equivalent (watch channel). Aborts at next statement boundary.

### Execution Protection

```rust
pub struct ExecutionLimits {
    pub max_call_depth: u32,       // default: 50
    pub max_command_count: u32,    // default: 10000
    pub max_loop_iterations: u32,  // default: 10000
}
```

### Output

```rust
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub command_count: u32,        // number of commands executed
}
```

## Security Model

1. **Network off by default**. No `curl`, no `wget`. Network access requires explicit configuration with URL allow-lists.
2. **No arbitrary binary execution**. Only built-in commands. No `./script.sh`, no `python script.py` (unless explicitly enabled).
3. **Path traversal protection**. All filesystem implementations validate paths to stay within their root.
4. **Execution limits**. Configurable limits prevent infinite loops and runaway commands.
5. **Timeout**. Hard timeout kills execution at statement boundary, returns partial output.

## Implementation Plan

### Phase 1 (v1 core)
- `FileSystem` trait + `InMemoryFs`
- Shell parser (tokenizer, AST, executor) for v1 syntax subset
- `CommandRegistry` + trait
- All v1 commands listed above
- `BashSession` with persistent state
- Integration into the bash tool

### Phase 2 (v1.1)
- `OverlayFs` and `ReadWriteFs`
- `MountableFs`
- Additional commands: `find`, `tee`, `printf`, `sleep`, `timeout`

### Phase 3 (post-v1)
- `curl` with network allow-list
- `sed`, `awk`, `diff`
- WASM runtimes for Python/JS
- `jq`, `yq`, `sqlite3`
