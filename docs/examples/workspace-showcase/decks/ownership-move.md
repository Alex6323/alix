---
title: "Rust ownership moves"
description: A guided predict-and-verify walk through real source.
trace: How `let s2 = s1` moves a String and avoids a double free.
source: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md
format-version: 1
id: "deck-5w9g21vjyf3xf3kpn9q8cckavs"
---

<!-- Frozen excerpts from "The Rust Programming Language" (github.com/rust-lang/book). -->
<!-- Chapter 4 "Understanding Ownership", dual MIT/Apache-2.0. -->
<!-- Frozen under this deck's own asset directory, so the example stays walkable offline. -->

## A `String` lives partly on the stack and partly on the heap. What are the three parts kept on the stack, and where do the contents live?
Stack: a pointer, a length, and a capacity.
Heap: the actual character contents.
> Length is bytes currently used; capacity is bytes received from the allocator.
<!-- at: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md fingerprint: xxh64-fbc4c018148dac41 asset: sha256-0e01969ffa5628cec10f139bca4404df9debfc317b43cd7fc1ce7c4f1af527cc.md -->
<!-- id: card-4mwwdfwyeb9nvsm2x03rchknj9 -->

## Given that layout, when you write `let s2 = s1`, what exactly gets copied?
Only the stack data (pointer, length, capacity) is copied.
The heap contents are not copied; `s1` and `s2` point at the same heap data.
<!-- at: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md fingerprint: xxh64-16503ce6bd047bfc asset: sha256-9e9c232518a9b39891857b365a9e85239509da9822d3691a26f2f7ade5987284.md -->
<!-- id: card-0c76yd1ta5h68bhfk8mbb2fzcn -->

## If both `s1` and `s2` pointed at the same heap data and both went out of scope, what memory bug would occur?
A double free: both would call `drop` on the same memory, risking corruption.
<!-- at: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md fingerprint: xxh64-684012cb0bf8b6c9 asset: sha256-1369019b16981c7381f621fb0e73ca4b48b4276de392f0059529936ceb73ded2.md -->
<!-- id: card-23wyjq03gnq8t81qyp0cbh0qda -->

## So how does Rust prevent that double free after `let s2 = s1`?
It treats the assignment as a move: `s1` is considered no longer valid, so only `s2` frees the memory.
Using `s1` afterward is a compile-time error.
> A move is a shallow copy (pointer, length, capacity) plus invalidation of the source.
<!-- at: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md fingerprint: xxh64-e493709594baa752 asset: sha256-2ca3a57a6a89ad30fedd2b25f73203997948a59721215df1d59ee4d666e64bbe.md -->
<!-- id: card-7778gyq63jd49h6yrbw8w5q8m6 -->

## Does this mean Rust ever silently makes a deep copy of heap data?
No. Rust never automatically deep-copies, so any automatic copy can be assumed cheap.
<!-- at: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md fingerprint: xxh64-c09ba7870cc043c2 asset: sha256-a66036d9b644edb49b6e851bdb1417841e0295acd252e277d885d17594e0a5dd.md -->
<!-- id: card-3dyw29w4q19avx46bj6a9wy0nd -->
