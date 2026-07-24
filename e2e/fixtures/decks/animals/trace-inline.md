---
id: "00000000000000000000000001"
trace: How `push` grows a `Vec`
source: trace-source.txt
---

# Inline Trace

## When `push` finds no spare capacity, what happens next?
<!-- given: `Vec` — the collection being grown -->
It calls `reserve` before inserting the element.
> `reserve` establishes capacity first.
<!-- at: 1 -->
<!-- id: 00000000000000000000000002 -->
