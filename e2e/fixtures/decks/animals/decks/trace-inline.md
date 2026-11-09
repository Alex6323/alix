---
id: "deck-00000000000000000000000001"
trace: How `push` grows a `Vec`
exam: trace-source.txt
cite:
  - src: .
title: Inline Trace
---

## When `push` finds no spare capacity, what happens next?
It calls `reserve` before inserting the element.
> [!NOTE]
> `reserve` establishes capacity first.
<!-- given: `Vec` — the collection being grown -->
<!-- at: src:trace-source.txt:1 fingerprint: 7b42tasakgn1f asset: asset-0xseft91rj11v2nkss7vx3m6g3gxgbv4hcg82152c75yya14r3eq.txt -->
<!-- id: card-00000000000000000000000002 -->
