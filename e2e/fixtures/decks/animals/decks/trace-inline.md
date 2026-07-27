---
alix-id: "00000000000000000000000001"
trace: How `push` grows a `Vec`
source: assets/00000000000000000000000001/sha256-772e7e921c4821d8ab3ce4fbe8e8680e1d82f648b208104a261cbef2824c0dd7.txt
origin: trace-source.txt
---

# Inline Trace

## When `push` finds no spare capacity, what happens next?
<!-- given: `Vec` — the collection being grown -->
It calls `reserve` before inserting the element.
> `reserve` establishes capacity first.
<!-- at: sha256-772e7e921c4821d8ab3ce4fbe8e8680e1d82f648b208104a261cbef2824c0dd7.txt:1 @ xxh64:75905a565538542f from trace-source.txt:1 -->
<!-- id: 00000000000000000000000002 -->
