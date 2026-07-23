---
id: "5w9g21vjyf3xf3kpn9q8cckavs"
trace: How `let s2 = s1` moves a String and avoids a double free.
source: assets
---

# Rust ownership moves

A guided predict-and-verify walk through real source.

Frozen excerpts from "The Rust Programming Language" (github.com/rust-lang/book).
Chapter 4 "Understanding Ownership", dual MIT/Apache-2.0.
Snapshotted under assets so this example stays valid and walkable offline.

## A `String` lives partly on the stack and partly on the heap. What are the three parts kept on the stack, and where do the contents live?
Stack: a pointer, a length, and a capacity.
Heap: the actual character contents.
> Length is bytes currently used; capacity is bytes received from the allocator.
<!-- at: 01.md -->
<!-- id: 4mwwdfwyeb9nvsm2x03rchknj9 -->

## Given that layout, when you write `let s2 = s1`, what exactly gets copied?
Only the stack data (pointer, length, capacity) is copied.
The heap contents are not copied; `s1` and `s2` point at the same heap data.
<!-- at: 02.md -->
<!-- id: 0c76yd1ta5h68bhfk8mbb2fzcn -->

## If both `s1` and `s2` pointed at the same heap data and both went out of scope, what memory bug would occur?
A double free: both would call `drop` on the same memory, risking corruption.
<!-- at: 03.md -->
<!-- id: 23wyjq03gnq8t81qyp0cbh0qda -->

## So how does Rust prevent that double free after `let s2 = s1`?
It treats the assignment as a move: `s1` is considered no longer valid, so only `s2` frees the memory.
Using `s1` afterward is a compile-time error.
> A move is a shallow copy (pointer, length, capacity) plus invalidation of the source.
<!-- at: 04.md -->
<!-- id: 7778gyq63jd49h6yrbw8w5q8m6 -->

## Does this mean Rust ever silently makes a deep copy of heap data?
No. Rust never automatically deep-copies, so any automatic copy can be assumed cheap.
<!-- at: 05.md -->
<!-- id: 3dyw29w4q19avx46bj6a9wy0nd -->
