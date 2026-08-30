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

### H2 — `allocate_cluster`'s doc describes three paths, the code has two — **fixed**

The doc now describes two passes and a refusal, which is what the code does. The
"third path" was a garbled account of the failure case: if pass 1 finds nothing
free *and* there is no empty refcount-table slot either, the table itself is
full, growing it is out of scope, and the call returns `Unsupported`. It never
"recycled cluster 0's spare scan", which is what the old prose seemed to claim.

### H3 — the public `write_at` contract contradicted the implementation and itself — **fixed**

The C header still described "Phase A constraints": writes succeeding only
against already-allocated, single-reference, uncompressed clusters, with
allocation returning `FS_CORE_CUSTOM`. The code allocates, maintains refcounts,
and copies up from the backing chain.

`include/qcow2.h` now states what actually happens, and what is still refused
(snapshots, `refcount_order != 4`, full refcount blocks) — plus the one thing no
document said: a write to a **compressed** cluster allocates an uncompressed one
in its place rather than re-compressing.

### H4 — the compressed-cluster descriptor decode is unnamed, and encode re-derives it — **fixed, and it was hiding three dead branches**

Every quantity now has a name: `offset_bits` (the field width, which *moves with
the cluster size* — a bigger cluster needs more bits for the sector count, so it
leaves fewer for the offset), `additional_sectors`, `first_sector_start`, and
`COMPRESSED_SECTOR_SIZE` — which the doc says is deliberately neither the cluster
size nor the device's block size.

The naming pass was the cheap half. Measuring first showed the decode was
**effectively untested**:

| mutation | before | after |
|---|---|---|
| offset-field width `- 8` → `- 9` | 0 tests fail | 3 |
| `COMPRESSED_SECTOR_SIZE` 512 → 1024 | 0 | 3 |
| `(additional + 1)` → `additional` | 0 | 7 |

Every compressed fixture is a cluster of one repeated byte, which deflates to
well under one sector — so the sector count was always zero, and the whole
"additional sectors" half of the descriptor was dead as far as the tests were
concerned. Four unit tests now cover multi-sector payloads, unaligned host
offsets (the `- (host_off % 512)` term the report flagged), and cluster sizes
other than 4 KiB. Their encode side is written from the specification rather
than derived from the decoder, so the round trip is a check and not a tautology.

### H5 — the two-level address translation is recomputed inline in four places — **fixed, and half of it was untested**

`ClusterAddress { virt_cluster, l1_index, l2_index, offset_in_cluster }` and
`split_virtual`. The rule "one L1 entry covers `l2_entries` clusters" is now
stated once instead of asserted at every site that needed an address — in two
different spellings, which is what made the write path's branches read as three
unrelated paragraphs.

**`l1_index` was dead code as far as the tests were concerned.** Forcing it to
zero failed *no* tests: one L1 entry covers 2 MiB at a 4 KiB cluster, and every
fixture in the suite was 16 KiB, so the second level of the crate's two-level
addressing had never been exercised. `build_two_l1_entry_image` builds a 3 MiB
image with two L1 entries and two L2 tables; two tests read and write past the
2 MiB boundary and check the *other* table stayed untouched. The mutation now
fails 2.

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

### M1, M5, M6, M7, M8, M9 — duplication and bare literals — **fixed**

- **M1 — the refcount-width guard, and it was completely uncovered.** Deleting
  the whole `refcount_order != 4` check failed **no** tests. It is not a
  politeness: every refcount walk treats blocks as arrays of big-endian `u16`,
  and at any other order that stride reads two neighbouring entries as one,
  hands out a cluster that is in use, and overwrites live data with it. One
  `refcount_entries_per_block()` now owns it, and
  `a_non_u16_refcount_width_is_refused_rather_than_mis_walked` covers it.
- **M5** — `TABLE_ENTRY_BYTES` and `REFCOUNT_ENTRY_BYTES` replace `8` and `2`
  across thirteen sites where either could have been a byte count, a bit width
  or an alignment.
- **M6** — `header::offsets`, with the same call as vhd's H7: the parser reads
  through the constants, **the fixtures deliberately keep their literals**.
  Moving `CLUSTER_BITS` by a byte fails 52 tests and `L1_TABLE_OFFSET` 35,
  precisely because the fixtures were written from the specification and do not
  import the table. `offsets_match_the_published_specification` and
  `no_header_field_overlaps_its_neighbour` write that intent down.
- **M7** — one `build_compressed_image_with(path, pattern, Compressor)`. The two
  builders differed in the compressor call and three header bytes; the other
  ~65 lines were identical.
- **M8** — one `HOST_OFFSET_MASK` per *side* rather than eleven copies on one:
  the crate has `OFFSET_MASK`, the fixtures have their own, and they are meant
  to be able to disagree. A new derived test pins the crate's to bits 9..55 —
  widening it to strip bit 9 failed no tests before, since every fixture offset
  is cluster-aligned.
- **M9** — `l2_entry_for(host_off)`. Dropping its mask also failed **no** tests
  for the same reason, so a test now covers the case the allocator never
  produces: an offset with low bits set would put those bits on top of the flags
  field, and on a v3 image bit 0 is the "reads as zeros" flag.

### M3, M4 — `write_at` and `allocate_cluster` are god functions (129 and 104 lines) — **needs your decision**

Both are the paths that establish the crate's crash-safety ordering. Splitting
them is defensible; doing it without a test on the seams is not.

### M11 — unnamed header-size thresholds in the open path — **needs your decision**

Small, but the thresholds encode which header layout is being assumed, and
naming them means deciding what the names assert.

### M13 — the two caches encode their key invariant only in a comment — **needs your decision**

The fix is to encode it in a type, which changes the cache's shape.

---

## Verification (updated)

**67 tests pass, up from 58.** `cargo clippy --all-targets -- -D warnings` clean.
Nine of the new tests exist because a mutation showed the code they cover was
unreachable from the suite; the details are under each finding above.

---

## Verification

56 tests pass across all binaries — the new one being the header
cluster regression. `chore lint` clean.
