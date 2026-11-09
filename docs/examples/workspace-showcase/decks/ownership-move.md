---
title: "Rust ownership moves"
description: A guided predict-and-verify trace through real source.
trace: How `let s2 = s1` moves a String and avoids a double free.
exam: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md
cite:
  - src: https://github.com/rust-lang/book/blob/main/src/ch04-01-what-is-ownership.md
id: "deck-5w9g21vjyf3xf3kpn9q8cckavs"
---

<!-- Frozen excerpts from "The Rust Programming Language" (github.com/rust-lang/book). -->
<!-- Chapter 4 "Understanding Ownership", dual MIT/Apache-2.0. -->
<!-- Frozen under this deck's own asset directory, so the example traces offline. -->

## A `String` lives partly on the stack and partly on the heap. What are the three parts kept on the stack, and where do the contents live?
Stack: a pointer, a length, and a capacity.
Heap: the actual character contents.
> [!NOTE]
> Length is bytes currently used; capacity is bytes received from the allocator.
<!-- at: src fingerprint: fqh6030a8vb21 asset: asset-03g1jtfzmnh8sv0gy4wvs9209qwxxfy32yt3snzw3kkw9wdfa9yc.md -->
<!-- id: card-4mwwdfwyeb9nvsm2x03rchknj9 -->

## Given that layout, when you write `let s2 = s1`, what exactly gets copied?
Only the stack data (pointer, length, capacity) is copied.
The heap contents are not copied; `s1` and `s2` point at the same heap data.
<!-- at: src fingerprint: 1cm1wwtyg8yzw asset: asset-17mw4cjhhadkk28rayspbaf8a8wn17d9g8pkd4d2dwqqnqjsgwm4.md -->
<!-- id: card-0c76yd1ta5h68bhfk8mbb2fzcn -->

## If both `s1` and `s2` pointed at the same heap data and both went out of scope, what memory bug would occur?
A double free: both would call `drop` on the same memory, risking corruption.
<!-- at: src fingerprint: 6gg0jsc5zhdp9 asset: asset-04v906dhd60wee0zc8fv1sswmjt8pgkpvrwjy02saackdknq7qpj.md -->
<!-- id: card-23wyjq03gnq8t81qyp0cbh0qda -->

## So how does Rust prevent that double free after `let s2 = s1`?
It treats the assignment as a move: `s1` is considered no longer valid, so only `s2` frees the memory.
Using `s1` afterward is a compile-time error.
> [!NOTE]
> A move is a shallow copy (pointer, length, capacity) plus invalidation of the source.
<!-- at: src fingerprint: e94vgjpabn9tj asset: asset-0b53mnx6n2dd63zdtas5yws076bs92jse891bqrxb7q4tskecjxy.md -->
<!-- id: card-7778gyq63jd49h6yrbw8w5q8m6 -->

## Does this mean Rust ever silently makes a deep copy of heap data?
No. Rust never automatically deep-copies, so any automatic copy can be assumed cheap.
<!-- at: src fingerprint: c16x7gw6c0gy2 asset: asset-19k06vcvch7dpjdpx18vvca1f10y0aatsmjjw9vxh1ehepae19ex.md -->
<!-- id: card-3dyw29w4q19avx46bj6a9wy0nd -->
