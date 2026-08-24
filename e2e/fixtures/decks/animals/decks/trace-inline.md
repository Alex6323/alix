---
id: "deck-00000000000000000000000001"
trace: How `push` grows a `Vec`
source: trace-source.txt
title: Inline Trace
---

## When `push` finds no spare capacity, what happens next?
It calls `reserve` before inserting the element.
> [!NOTE]
> `reserve` establishes capacity first.
<!-- given: `Vec` — the collection being grown -->
<!-- at: trace-source.txt:1 fingerprint: xxh64-75905a565538542f asset: sha256-772e7e921c4821d8ab3ce4fbe8e8680e1d82f648b208104a261cbef2824c0dd7.txt -->
<!-- id: card-00000000000000000000000002 -->
