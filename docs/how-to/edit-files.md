# How to Edit Files with Hash Anchors

This guide covers the edit workflow: reading files with hash anchors and applying precise edits.

## Read a file first

Always read a file before editing it. The `read` tool returns each line with a `LINE#HASH:` prefix:

```
 8#VR:function hello() {
 9#KT:  console.log("world");
10#BH:}
```

The hash (`VR`, `KT`, `BH`) is a 2-character digest of the line content. You reference these hashes when editing.

## Apply a single edit

Use an edit operation with the `pos` anchor from the read output:

```json
{
  "path": "src/main.rs",
  "edits": [
    { "op": "replace", "pos": "9#KT", "lines": ["  console.log(\"hackpi\");"] }
  ]
}
```

The edit replaces line 9 whose hash is `KT`. If the file has changed since the read and the hash no longer matches, the edit is rejected.

## Replace a range

Use `pos` and `end` to replace multiple lines at once:

```json
{ "op": "replace", "pos": "8#VR", "end": "10#BH", "lines": ["function hello() {", "  console.log(\"hackpi\");", "}"] }
```

## Insert lines after a position

```json
{ "op": "append", "pos": "10#BH", "lines": ["", "function goodbye() {", "  console.log(\"bye\");", "}"] }
```

Omit `pos` to append at the end of the file.

## Insert lines before a position

```json
{ "op": "prepend", "pos": "8#VR", "lines": ["// greeting module", ""] }
```

Omit `pos` to prepend at the beginning of the file.

## Replace an exact substring

For targeted text replacement without line anchors:

```json
{ "op": "replace_text", "oldText": "console.log(\"world\")", "newText": "console.log(\"hackpi\")" }
```

This fails if `oldText` is not found or matches more than once.

## Chain multiple edits

All edits in one call validate against the same snapshot, then apply bottom-up so line numbers stay consistent:

```json
{
  "path": "src/main.rs",
  "edits": [
    { "op": "replace", "pos": "9#KT", "lines": ["  console.log(\"hackpi\");"] },
    { "op": "append", "pos": "10#BH", "lines": ["", "// end of module"] }
  ]
}
```

After a successful edit, the result includes an `--- Updated anchors ---` block with fresh hashes. Use those for subsequent edits on the same file without a full re-read.

## Common errors

| Error | Cause | Fix |
|-------|-------|-----|
| `[E_STALE_ANCHOR]` | File changed since last read | Re-read the file and use the new hashes |
| `[E_INVALID_PATCH]` | Lines contain `LINE#HASH:` prefixes or diff `+`/`-` markers | Send literal file content only |
| `File not found` | Path does not exist | Check the path |

## Further reading

- [Tools reference](../reference/tools.md) for full edit tool schema
- [Hash anchors explanation](../explanation/hash-anchors.md) for how hashes work and why they matter