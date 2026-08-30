# Human-code findings — status

Tracks every **High** and **Medium** finding from
[`human-code-report-2026-08-28.md`](human-code-report-2026-08-28.md). The report
predates the work; this is the current position. Updated 2026-08-30.

**25 findings** — 5 High, 13 Medium, 7 Low. This covers the 18 High and Medium.

| | High | Medium |
|---|---|---|
| Fixed | 2 | 2 |
| Left for a human decision | 1 | 5 |
| Fixable, not yet done | 2 | 6 |

---

## The one that could destroy an image

**M12 — nothing forbade the allocator from handing out host cluster 0.**

`allocate_cluster` scans from block 0, entry 0. Host cluster 0 holds the
**header**. A well-formed image always marks it in use, so the scan never
reaches it — but this crate reads images it does not trust, and a malformed one
that leaves that refcount at zero was handed cluster 0. The caller immediately
zero-fills what it is given, wiping the header; the L2 entry written afterwards
is `0 | COPIED`, which `lookup_cluster` reads straight back as `Unallocated`, so
the write is lost too.

Now refused with `refcount table marks host cluster 0 (the header) as free`.
`allocator_refuses_the_header_cluster` builds that image deliberately — no
fixture reached it, because every fixture is well-formed — and fails against the
unguarded code with `must not hand out the header cluster: ()`, the `()` being
the `Ok` it used to return.

---

## High

### H1 — `write_at` never consulted the backing chain — **fixed earlier**

[#23](https://github.com/antimatter-studios/rust-img-qcow2/pull/23) — the merged
`Zero | Unallocated` arm was discarding a parent image's data on write. Copy-up
now happens.

### H2 — `allocate_cluster`'s doc describes three paths, the code has two — **fixable, not yet done**

The prose for the third path is incoherent, and rewriting it needs the intended
behaviour established rather than guessed. Grouped with H4/H5.

### H3 — the public `write_at` contract contradicted the implementation and itself — **fixed**

The C header still described "Phase A constraints": writes succeeding only
against already-allocated, single-reference, uncompressed clusters, with
allocation returning `FS_CORE_CUSTOM`. The code allocates, maintains refcounts,
and copies up from the backing chain.

`include/qcow2.h` now states what actually happens, and what is still refused
(snapshots, `refcount_order != 4`, full refcount blocks) — plus the one thing no
document said: a write to a **compressed** cluster allocates an uncompressed one
in its place rather than re-compressing.

### H4 — the compressed-cluster descriptor decode is unnamed, and encode re-derives it — **fixable, not yet done**

Real, and the fix is a shared descriptor type. Worth its own change.

### H5 — the two-level address translation is recomputed inline in four places — **fixable, not yet done**

Same batch as H4.

---

## Medium

### M10 — module docs list supported features as unsupported — **fixed**

`lib.rs` and `reader.rs` both said zstd was "not yet", and neither mentioned the
write path at all. Both are wrong: `ruzstd` is a dependency, the dispatch is at
`reader.rs:1150`, and `open_rw`/`write_at` have been there since Phase A.

Corrected, including what is genuinely still missing.

### M12 — the allocator could hand out the header cluster — **fixed**, see above.

### M2 — `decrement_refcount` and `read_refcount` were thirty near-identical lines — **fixed**

Both locate a cluster's refcount entry the same way: width check, entries per
block, block and entry index, table bounds, block pointer. Then they diverge —
and **the divergence is correct**, which is what made this worth care rather than
a mechanical merge:

| when the entry is absent | `read_refcount` | `decrement_refcount` |
|---|---|---|
| past the table's coverage | `Ok(0)` | `Corrupt` |
| block not allocated | `Ok(0)` | `Corrupt` |

Reading a refcount for an uncovered cluster legitimately means *no references* —
the cluster is not live, so there is no share to copy away from. Decrementing one
means the caller is releasing something never recorded as taken.

A reader had to diff the two functions to discover that. `locate_refcount_entry`
now returns a `RefcountEntryLocation` with the absent cases as **separate
variants**, and each caller answers them in its own words. Nothing got vaguer in
order to be shared, which is the line worth holding.

**The probe found a real coverage gap**, recorded here rather than papered over.
Changing `entries_per_block` from `cluster_size / 2` to `/ 4` in
`decrement_refcount` failed **nothing**: every fixture is small enough that
`host_cluster_idx < entries_per_block`, so `block_idx` is always 0 and the
divisor never shows. **No test image spans two refcount blocks**, and reaching
one needs a fixture over 2048 clusters. The consolidation at least means the two
functions can no longer disagree about it — mutating the shared arithmetic now
fails 3 tests where the duplicated copy failed 0.

### M1, M5, M6, M7, M8, M9 — duplication and bare literals — **fixable, not yet done**

The refcount-width preamble three times; `8` as the table-entry size at 13 sites;
header offsets open-coded six ways; the two fixture builders 95% identical;
`OFFSET_MASK`'s value written longhand 11 times; and the
`(host & MASK) | COPIED` construction four times.

### M3, M4 — `write_at` and `allocate_cluster` are god functions (129 and 104 lines) — **needs your decision**

Both are the paths that establish the crate's crash-safety ordering. Splitting
them is defensible; doing it without a test on the seams is not.

### M11 — unnamed header-size thresholds in the open path — **needs your decision**

Small, but the thresholds encode which header layout is being assumed, and
naming them means deciding what the names assert.

### M13 — the two caches encode their key invariant only in a comment — **needs your decision**

The fix is to encode it in a type, which changes the cache's shape.

---

## Verification

56 tests pass across all binaries — the new one being the header
cluster regression. `chore lint` clean.
