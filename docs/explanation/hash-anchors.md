# Why Hash Anchors?

Most coding agents edit files by matching raw text. When the file changes between the read and the edit, these agents silently relocate the match — applying the edit to the wrong line.

hackpi takes a different approach: every line returned by `read` includes a short hash of its content. Edits reference these hashes. If the file changes and the hash no longer matches, the edit is rejected outright rather than applied to the wrong place.

## The problem with text matching

Consider this workflow:

1. The agent reads `main.rs` and sees `console.log("world")` on line 9
2. Meanwhile, another edit changes line 9 to `console.log("updated")`
3. The agent tries to replace `console.log("world")` — but that text no longer exists, or it exists somewhere else in the file

Traditional agents handle this by finding the "closest match" and applying the edit there. This is a silent relocation: the edit lands on the wrong line, and neither the agent nor the user is told.

## How hash anchors work

The `read` tool prefixes each line with `LINE#HASH:`:

```
 8#VR:function hello() {
 9#KT:  console.log("world");
10#BH:}
```

When the agent edits, it references line 9 by its hash `KT`:

```json
{ "op": "replace", "pos": "9#KT", "lines": ["  console.log(\"hackpi\");"] }
```

The edit tool checks: does line 9 currently have hash `KT`? If yes, the edit applies. If no, the edit is rejected with fresh hashes for the affected lines so the agent can retry immediately.

## Design decisions

**No fallback relocation.** A mismatched hash is never silently moved to a "close enough" line. Correctness is more important than convenience.

**Two-character alphabet.** Hashes use characters from `ZPMQVRWSNKTXJBYH` — excluding hex digits, vowels, and visually ambiguous letters. This means a reference like `5#MQ` cannot be confused with code content, hex literals, or English words.

**Deterministic hashing.** The xxHash32 algorithm is fast and deterministic. Lines with no alphanumeric characters (like a lone `}`) use their line number as the hash seed to reduce collisions on structurally identical markers.

## Bottom-up application

When multiple edits are in one call, they apply bottom-up (highest line numbers first). This keeps earlier line numbers stable across operations. All edits validate against the same pre-edit snapshot, so the agent gets consistent results.

## Chained edits

After a successful edit, the tool returns an `--- Updated anchors ---` block with fresh hashes for the changed region. The agent can use these directly for subsequent edits on the same file without re-reading the whole file.

For changes far from the edited region, a full `read` is still recommended to get accurate hashes.