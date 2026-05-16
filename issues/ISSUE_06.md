# [Architecture] - [HIGH] - Seq (;) operator discards stderr redirects and cancel state from left operand

**Labels:** `bug`, `priority-high`, `architecture`

## Description

In `session.rs:304-309`, the `AstNode::Seq` handler runs the left command, then the right command, passing the same `stdout` and `stderr` buffers to both. However, the exit code of the Seq node is the exit code of the **right** operand only (line 309 returns `self.execute_node(right, ...)` without storing the left's exit code).

More critically, if the left command has stderr redirects (e.g., `cd /nonexistent 2>/tmp/err.txt; echo ok`), the stderr for the left command is written to the shared `stderr` buffer and the `2>/tmp/err.txt` redirect on the simple command properly captures it. But the cancel check after the left command (line 305-308) is present only in Seq, not in the redirect handling itself. The redirect to file is handled inside `execute_node` at the `AstNode::Simple` level, so the ordering is:
1. Left command runs with redirects
2. Cancel check (correct)
3. Right command runs

The actual issue is that the `Seq` node doesn't propagate the left operand's exit code. `cmd1; cmd2` should conceptually return `cmd2`'s exit code (which it does), but `cmd1; cmd1_fails` with no `cmd2` should return `cmd1`'s exit code.

## Location

- `hackpi-tools/src/bash/session.rs:304-310` — Seq execution

## Impact

- `false; echo ok` returns exit code 0 (from echo), which is correct per POSIX
- `false; false` also returns 0 (from second false), which is also correct — but if the user cares about the first command's failure, it's lost
- The test `test_seq_operator` only tests the happy path

## Proposed Solutions

1. Store the left exit code and return the last non-zero exit code if no right operand exists, or the right operand's exit code if it exists (matching `sh` semantics where Seq returns the last command's exit code)
2. Add tests for `false; echo ok`, `echo ok; false`, and `false; false` sequences
