# Hashline Edit Spec (v1)

Hash-anchored line editing for the hackpi coding agent. Inspired by [oh-my-pi](https://github.com/can1357/oh-my-pi) and [pi-hashline-edit](https://github.com/RimuruW/pi-hashline-edit).

## Core Concept

Every line returned by `read` carries a short content hash. Edits reference these hashes instead of raw text, so the tool can detect stale context and reject outdated changes before they reach the file.

## Read Output Format

Text files are returned with a `LINE#HASH:` prefix on every line. Line numbers are left-padded so `#HASH:` columns align:

```
 8#VR:function hello() {
 9#KT:  console.log("world");
10#BH:}
```

- `LINE` — 1-indexed line number.
- `HASH` — 2-character content hash from alphabet `ZPMQVRWSNKTXJBYH`.

### Parameters

- `offset` — start reading from this line number (1-indexed).
- `limit` — maximum number of lines to return.

### Edge cases

- Images (JPEG, PNG, GIF, WebP): passed through as attachments, no hashline prefix.
- Binary files: rejected with descriptive error.
- Directories: listed as `type  name` entries, no hashline prefix.
- Empty files: return advisory suggesting `prepend`/`append` instead of a synthetic anchor.

## Edit Operations

Edits use `LINE#HASH` anchors from `read` output to target lines precisely:

```json
{
  "path": "src/main.rs",
  "edits": [
    { "op": "replace", "pos": "8#VR", "lines": ["fn hello() {"] }
  ]
}
```

### Operations

| Op | Purpose | Fields |
|---|---|---|
| `replace` | Replace one line (`pos`) or an inclusive range (`pos` + `end`) | `pos` required, `end` optional, `lines` |
| `append` | Insert lines after `pos`. Omit `pos` to append at EOF. | `pos` optional, `lines` |
| `prepend` | Insert lines before `pos`. Omit `pos` to prepend at BOF. | `pos` optional, `lines` |
| `replace_text` | Replace an exact unique substring anywhere in the file. Fails if text is not found or matches more than once. | `oldText`, `newText` |

### Execution

All edits in a single call validate against the same pre-edit snapshot and apply bottom-up, so line numbers stay consistent across operations.

### Chained edits

After a successful edit, the result includes an `--- Updated anchors ---` block with fresh `LINE#HASH` references for the changed region. These can be used directly in the next `edit` call on the same file without a full re-read. For distant changes, use `read` first.

### Diff preview

Each edit result includes a compact `Diff preview:` block showing changed lines with `+`/`-` markers and new `LINE#HASH` anchors.

## Hashing

Hash algorithm: xxHash32 via the `xxhash-rust` crate.

Mapping: xxHash32 output (u32) → 2 characters from alphabet `ZPMQVRWSNKTXJBYH`.

The alphabet excludes hex digits, common vowels, and visually ambiguous letters (D/G/I/L/O), so a reference like `5#MQ` can never be confused with code content, hex literals, or English words.

Lines with no alphanumeric characters (e.g. a lone `}`) use their line number as the hash seed to reduce collisions on structurally identical markers.

## Design Rules

1. **Stale anchors fail.** A hash mismatch means the file has changed since the last `read`. The error includes a snippet with fresh `LINE#HASH` references for the affected lines for immediate retry.

2. **No fallback relocation.** Mismatched anchors are never silently relocated to a "close enough" line. Correctness over convenience.

3. **Strict patch content.** If `lines` contains `LINE#HASH:` display prefixes or diff `+`/`-` markers, the edit is rejected with `[E_INVALID_PATCH]`. The model must send literal file content.

4. **Atomic writes.** Files are written via temp-file-then-rename to avoid corruption from interrupted writes. Symlink chains are resolved so the target file is updated without replacing the symlink. Hard-linked files are updated in place to preserve the shared inode. File permissions preserved across atomic renames.

5. **Per-file mutation queue.** Edits queue by canonical write target, so concurrent edits through different symlink paths serialize onto the same underlying file.

## Rust Implementation

- Crate: `xxhash-rust` for xxHash32
- Hash function maps `u32` → 2 chars via modulo 16 into the alphabet
- Line hashing operates on trimmed content (no `LINE#HASH:` prefix in hash computation)
- Read tool returns `String` with `LINE#HASH:` prefixes
- Edit tool parses `LINE#HASH` references, verifies hash matches current file content
