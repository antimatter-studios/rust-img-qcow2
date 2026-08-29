# Human-code report — am-img-qcow2

**Date:** 2026-08-28
**Scope:** full crate (`src/`, `tests/`, `examples/`, `include/`) — v0.4.2, 4,020 lines
**Status:** **ANALYSIS ONLY — no code was changed.** No files other than this
report were created or modified; no branch, no commit. Phases 0, 1 and 3 of the
human-code skill were run; Phase 2 (dev-loop / implementation) was deliberately
skipped pending your review.

| | Count |
|---|---|
| Items found | 25 |
| High | 5 |
| Medium | 13 |
| Low | 7 |
| Items fixed | 0 (report-only run) |
| Items skipped | 0 (nothing was actioned; see *Considered and not flagged* for what was deliberately **not** raised) |

Baseline at time of scan: **53 tests passing, 0 failing** (23 lib + 30
integration), plus 15 `qemu-validation`-gated tests not run by default.
`cargo clippy --all-targets` is clean. Working tree was clean before and after.

---

## Findings

### High

---

#### H1 — `write_at` never consults the backing chain; the merged `Zero | Unallocated` arm silently discards a parent image's data

- **File:** `src/reader.rs:541-554` (the write branch), contrast `src/reader.rs:948-972` (`read_unallocated`)
- **Category:** Duplicated code hiding a divergence / merged match arm with two different contracts
- **Severity:** High — silent data loss, not a readability nit
- **Test coverage:** **none.** Every write test (`tests/synthetic.rs:212-578, 655-766`) uses a fixture with no backing file; every backing test (`tests/synthetic.rs:148-205`) is read-only. `grep -n backing src/reader.rs` returns no hit anywhere between lines 433 and 561.

`write_at` has three near-identical blocks that all mean "get a fresh host
cluster, seed it, splice the caller's bytes in, repoint L2":

```rust
ClusterMap::Plain { .. } if needs_cow => {
    let mut full = vec![0u8; cluster_size as usize];
    self.dev_read(host_off, &mut full)?;              // seed from the old cluster
    full[in_cluster..in_cluster + chunk].copy_from_slice(src);
    ...
}
ClusterMap::Compressed { host_off, byte_len } => {
    let mut full = vec![0u8; cluster_size as usize];
    self.read_decompressed_slice(...)?;               // seed from the compressed cluster
    full[in_cluster..in_cluster + chunk].copy_from_slice(src);
    ...
}
ClusterMap::Zero | ClusterMap::Unallocated => {
    let new_host = self.allocate_cluster()?;
    self.zero_cluster(new_host)?;                     // seed from... nothing
    self.dev_write(new_host + in_cluster, src)?;
    ...
}
```

The first two seed the new cluster from the data it is replacing. The third
zero-fills. For `ClusterMap::Zero` that is correct — a v3 zero-flagged cluster
never defers to backing, and `read_unallocated` is not involved. For
`ClusterMap::Unallocated` on an image opened with `open_rw` that **has** a
backing parent, it is wrong: before the write, reads of that cluster fell
through to the parent (`read_unallocated`, line 950); after a 4-byte write, the
L2 entry points at a zero-filled cluster and the other `cluster_size - 4` bytes
of the parent's data are gone. QCOW2 requires copy-up of the untouched head and
tail from the backing chain when allocating on write.

Two structural choices made this easy to miss: the two states with *different*
backing semantics were merged into one match arm, and the three "allocate and
splice" blocks are copy-pasted rather than sharing a named
`seed_new_cluster(...)` step — so the odd one out reads as just another
paragraph of the same shape.

---

#### H2 — `allocate_cluster`'s doc comment describes three paths; the code has two, and the third path's prose is incoherent

- **File:** `src/reader.rs:563-585` (doc) vs `src/reader.rs:586-689` (code)
- **Category:** Comments that lie
- **Severity:** High — the allocator is the most safety-critical function in the write path and its only prose description is wrong
- **Test coverage:** the grow path is covered behaviourally by `tests/synthetic.rs:420-501` (`write_grows_refcount_block`); nothing checks the documented contract

The doc lists "Three fast paths in order", where path 3 reads:

> 3. Every populated block is full and there is no empty slot — we grow at the
>    tail by recycling cluster `host_cluster_idx == 0`'s spare scan: in practice
>    every refcount table sized for the underlying device has spare entries...

There is no such path in the body. The code has exactly two passes: scan every
present block for a free entry (lines 618-646), then grow into the first absent
refcount-table slot (lines 650-688). The "recycling cluster
`host_cluster_idx == 0`'s spare scan" clause does not correspond to any code and
cannot be reconciled with it. A reader trying to reason about allocation safety
has to discard the doc entirely and re-derive the behaviour from the bytes.

---

#### H3 — the public `write_at` contract contradicts the implementation *and itself*, and the same stale text is duplicated into the C header

- **Files:** `src/reader.rs:405-432` (rustdoc), `src/reader.rs:452-457` (inline comment), `src/capi.rs:30-34`, `include/qcow2.h:26-40`
- **Category:** Comments that lie / stale published API docs
- **Severity:** High — this is the documentation a downstream FFI consumer reads before deciding what the writer can be trusted with
- **Test coverage:** n/a (documentation)

Three concrete contradictions:

1. Rustdoc line 416-418: "The old compressed cluster's refcount is currently
   left in place — `qemu-img check` will report it as a leak; Phase D adds the
   decrement." Line 539 calls `self.decrement_refcount(host_off)`, and
   `tests/synthetic.rs:503-527` asserts the replacement behaviour.
2. Rustdoc line 426: "Refused with `Error::Unsupported`: Image with
   `nb_snapshots > 0`". The inline comment 70 lines below (452-457) explicitly
   says that check "is gone" and explains why. The doc and the comment
   describing the doc's removal live in the same function.
3. `capi.rs:30-34` and `include/qcow2.h:35-40` both still say writes succeed
   "only against already-allocated, single-reference, uncompressed clusters" —
   i.e. the Phase A contract — while the crate now allocates, grows refcount
   blocks, rewrites compressed clusters and does snapshot CoW. The README (which
   *is* current) says the opposite of the shipped C header.

---

#### H4 — every step of the compressed-cluster descriptor decode is unnamed, and the encode side re-derives the same formula by hand

- **Files:** `src/reader.rs:1110-1121`; encode-side twins at `tests/common/mod.rs:184-186` and `tests/common/mod.rs:341-342`
- **Category:** Dense, impenetrable expressions + magic numbers
- **Severity:** High — this is the single densest expression in the crate and it sits on the compressed read path
- **Test coverage:** exercised end-to-end by `compressed_cluster_round_trip`, `zstd_compressed_cluster_round_trip` and three `qemu-validation` tests — but only at `cluster_bits = 12` and only with a **sector-aligned** host offset, so the `- (host_off % 512)` term of the span calculation is never exercised

```rust
fn decode_compressed_descriptor(entry: u64, cluster_bits: u32) -> (u64, usize) {
    let descriptor = entry & ((1u64 << 62) - 1);
    let x = (62 - (cluster_bits - 8)) as u64;
    let host_off = descriptor & ((1u64 << x) - 1);
    let n = descriptor >> x;
    let start_sector = host_off / 512;
    let end_byte = (start_sector + n + 1) * 512;
    let span = (end_byte - host_off) as usize;
    (host_off, span)
}
```

I checked this against the spec formula and it is **correct**. But nothing in
the code says so: `x` is the *bit width of the offset field*, `n` is the
*count of additional 512-byte sectors beyond the first*, `512` is the
*compressed-sector size* (which is deliberately not `cluster_size` and not the
device block size), and `(1u64 << 62) - 1` is *"strip COPIED and COMPRESSED"*.
Six unnamed quantities, three of them one-letter.

The other half of the same format lives in the test builders, where the width is
hand-computed with the cluster_bits inlined as a literal:

```rust
// x = 62 - (cluster_bits - 8). For cluster_bits=12, x=58.
let x: u64 = 62 - (12 - 8);
```

— duplicated verbatim in both compressed builders. Encoder and decoder share no
constant, no function, and no name, so a change to `CLUSTER_SIZE` in the
fixtures would silently mis-encode rather than fail to compile.

---

#### H5 — the two-level address translation is recomputed inline in four places and never named

- **Files:** `src/reader.rs:505-506`, `531-532`, `550-551`, `978-979`; supporting arithmetic at `459-468`, `906-914`, `976`
- **Category:** Dense expressions + duplicated code
- **Severity:** High — this is the crate's central abstraction and it exists only as repeated arithmetic
- **Test coverage:** heavily exercised (all 30 integration tests), but the duplication itself is invisible to tests — every copy is currently identical, so no test would fail if one drifted

The virtual→host translation has four conceptual steps: virtual byte →
virtual cluster → (L1 index, L2 index) → host offset + in-cluster offset. Not
one of them has a name. Instead:

```rust
let virt_cluster = cursor / cluster_size;
let l1_idx = (virt_cluster / l2_entries) as u32;
let l2_idx = (virt_cluster % l2_entries) as u32;
```

appears three times inside `write_at` alone (once per match arm), a fourth time
in `lookup_cluster` as `l1_index`/`l2_index` (different spelling for the same
thing), with `in_cluster = cursor & cluster_mask` and `virt_cluster = virt /
cluster_size` open-coded in both the read loop and the write loop.

There is no `ClusterAddress { l1_index, l2_index, offset_in_cluster }` and no
`fn split_virtual(virt) -> ClusterAddress`, so the rule "an L1 entry covers
`l2_entries` clusters" is asserted six times and owned nowhere. This is the
structural reason H1 is hard to see: because each write branch re-derives the
address instead of receiving one, the three branches read as three independent
paragraphs rather than three variants of one operation with one differing step.

---

### Medium

---

#### M1 — the refcount-width preamble is copy-pasted three times

- **Files:** `src/reader.rs:586-599`, `702-714`, `769-781`
- **Category:** Duplicated code + magic numbers
- **Test coverage:** all three functions are exercised; the duplicated guard is not independently tested

`allocate_cluster`, `decrement_refcount` and `read_refcount` each open with the
identical seven lines: derive `refcount_bits` from `version`/`refcount_order`,
reject anything but 16, then compute `entries_per_block`. Three instances meets
the extraction threshold. The entry width appears as a named
`refcount_bytes: u64 = 2` in one copy and as a bare `cluster_size / 2` in the
other two — the same constant with and without a name, ten lines apart.

#### M2 — `decrement_refcount` and `read_refcount` are the same function with a different final step

- **Files:** `src/reader.rs:702-759` vs `769-810`
- **Category:** Duplicated code
- **Test coverage:** `read_refcount` via the CoW tests (`tests/synthetic.rs:655-705`); `decrement_refcount` via the same tests' refcount assertions

Both: reject non-16-bit refcounts → compute block/entry index → validate the
refcount table covers it → read the table entry → read the block → index the
u16. ~40 lines shared, then one reads and the other writes back. They even
disagree on the "block not allocated" case (one returns `Corrupt`, the other
returns `Ok(0)`) — a real and deliberate difference that is invisible because it
sits at the end of two walls of identical code.

#### M3 — `write_at` is a god function (129 lines)

- **File:** `src/reader.rs:433-561`
- **Category:** God function
- **Test coverage:** good (10 tests), which is exactly why splitting it is low-risk

Bounds checking, the chunking loop, and three complete allocation strategies —
each with its own multi-line crash-safety commentary — in one body. The
match arms want to be `write_in_place`, `cow_clone_then_write`,
`decompress_rewrite`, `allocate_then_write`, all sharing one
`repoint_l2(address, new_host)` tail.

#### M4 — `allocate_cluster` is a god function (104 lines)

- **File:** `src/reader.rs:586-689`
- **Category:** God function
- **Test coverage:** both passes covered (`write_to_unallocated_cluster_allocates_phase_b`, `write_grows_refcount_block`)

Guard + table load + full-table scan + inner block scan + claim-and-flush +
new-block construction + table publish. The "Pass 1"/"Pass 2" section comments
are the tell — each pass is a function.

#### M5 — `8` (bytes per table entry) is a bare literal at 13 sites

- **Files:** `src/header.rs:143`, `175`; `src/reader.rs:275`, `279`, `612`, `619`, `684`, `728`, `735`, `795`, `800`, `844`, `882`, `1030`
- **Category:** Magic numbers
- **Test coverage:** implicit everywhere

`cluster_size / 8`, `l2_idx * 8`, `block_idx * 8`, `l1_idx * 8`, `rt_size / 8`
all encode "a table entry is a big-endian u64". `docs/future.md` §4 already
notes that extended-L2 support changes this to 16 for L2 tables — a named
`L2_ENTRY_BYTES` / `TABLE_ENTRY_BYTES` would both explain the arithmetic and
mark, in advance, every site that refactor has to visit.

#### M6 — header field offsets are open-coded at six independent sites

- **Files:** `src/header.rs:84-140` (parse); fixtures re-encode the same table at `tests/common/mod.rs:103-120`, `203-213`, `279-291`, `360-372`, `src/capi.rs:169-176`, `tests/synthetic.rs:351-360`
- **Category:** Magic numbers + duplicated code
- **Test coverage:** the header parser is well covered (16 unit tests); the fixture writers are not covered as code

`header.rs` documents the layout beautifully in a module doc comment and then
uses raw offsets anyway (`read_u32(bytes, 20)`, `read_u64(bytes, 40)`). Six
places now hard-code "cluster_bits is at 20", "l1_table_offset is at 40",
"header_length is at 100" — one reader and five writers that must agree by
inspection. A `mod offsets` with `pub const CLUSTER_BITS: usize = 20;` etc.,
shared with the fixtures, makes the agreement mechanical.

#### M7 — the zlib and zstd fixture builders are 95% identical

- **Files:** `tests/common/mod.rs:168-245` and `tests/common/mod.rs:329-399`
- **Category:** Duplicated code
- **Test coverage:** n/a (test infrastructure)

~70 lines each; they differ in the compressor call, three header bytes
(`incompatible_features`, `header_length`, byte 104), and nothing else — the
descriptor encoding, layout arithmetic, L1/L2 writes, refcount table and
refcount block are byte-for-byte the same. Counting `build_image` and
`build_child_with_backing`, four builders hand-write the same header block.

#### M8 — `OFFSET_MASK`'s value is written out longhand 11 times outside the constant

- **Files:** `src/reader.rs:33` (the const), then literals at `src/capi.rs:181`, `186`; `tests/common/mod.rs:125`, `131`, `132`, `218`, `298`, `376`; `tests/synthetic.rs:365`, `371`, `684`, `685`, `697`
- **Category:** Magic numbers
- **Test coverage:** n/a

The mask is private to `reader.rs`, so tests and fixtures cannot use it and
retype `0x00ff_ffff_ffff_fe00` instead. Exporting it (or, better, exporting
`fn host_offset_of(entry: u64) -> u64` / `fn allocated_entry(host: u64) -> u64`)
would make the encode and decode sides agree by construction rather than by
copy-paste — the same class of split-brain as H4.

#### M9 — `(new_host & OFFSET_MASK) | L2_FLAG_COPIED` is built four times

- **Files:** `src/reader.rs:507`, `533`, `552`, `869`
- **Category:** Duplicated code / unnamed invariant
- **Test coverage:** covered indirectly by every write test

The invariant "an entry we just allocated always has refcount 1, therefore
COPIED is set" is real, load-bearing (H1's `needs_cow` check depends on it), and
stated only by repetition. One `fn freshly_allocated_entry(host_off) -> u64`
would name it once.

#### M10 — module docs list supported features as unsupported

- **Files:** `src/lib.rs:3-6`, `src/reader.rs:3-12`, `include/qcow2.h:26-31`
- **Category:** Comments that lie
- **Test coverage:** n/a

`lib.rs` says "Supported now: Uncompressed and zlib-compressed clusters" and
never mentions zstd or the entire write path. `reader.rs` goes further and lists
"non-zlib compression types (zstd)" under **"Not yet"** — 1,060 lines above the
`ruzstd` decoder that implements it. The C header repeats the same omission.

#### M11 — unnamed header-size thresholds in the open path

- **Files:** `src/reader.rs:261`, `264` (`got >= 72`), `267`, `1131` (`len > 1024`)
- **Category:** Magic numbers
- **Test coverage:** `rejects_v3_header_shorter_than_104_bytes` covers the parse-side guard; the `112`/`72` device-read tolerance and the 1024-byte backing-path cap have no direct test

`112` (read enough to capture `compression_type`), `72` (the v2 minimum header),
`104` (the v3 fixed-field end, in `header.rs`) and `1024` (backing-path sanity
cap) are all spec quantities living as bare literals across two files.

#### M12 — nothing forbids the allocator from handing out host cluster 0 (the header)

- **Files:** `src/reader.rs:632-644` (the claim), `src/reader.rs:545-547` (the caller that then zero-fills it)
- **Category:** Missing invariant / unnamed precondition
- **Test coverage:** none — every fixture marks clusters 0..=6 as in-use, so the path is unreachable in tests

Pass 1 scans from `block_idx = 0, entry_idx = 0`. If a malformed or
partially-initialised image has refcount 0 for host cluster 0, `allocate_cluster`
returns offset `0` and the caller immediately calls `zero_cluster(0)` — wiping
the header. It then writes an L2 entry of `0 | COPIED`, which `lookup_cluster`
reads straight back as `Unallocated` (line 1004). Well-formed images always mark
cluster 0 in use, so this is malformed-input-only, but the invariant
("cluster 0 is never allocatable") is nowhere stated or enforced.

#### M13 — the two caches encode their key invariant only in a comment

- **Files:** `src/reader.rs:170-175` (fields), `1014-1033` (`read_l2_entry`), `850-852` (invalidation)
- **Category:** Dense/implicit invariant
- **Test coverage:** exercised by the compressed-read and write-then-read tests; no test targets cache invalidation directly

`read_l2_entry` takes both `l1_index` and `l2_table_off` but keys the cache on
`l1_index` alone. That is correct *because* L1 fully determines the L2 table
offset and *because* `update_l2_entry` clears the cache after any L1/L2 change —
two facts held together by a one-line comment ("Single-slot L2 cache: (l1_index,
cluster bytes)") and by the reader's memory. A tiny `L2Cache` with
`get(l1_index) / put / invalidate` would put the rule in one place, and would
make it obvious that `l2_table_off` is a *load* parameter, not part of the key.

---

### Low

| ID | Item | File | Category |
|---|---|---|---|
| L1 | `hex_dump` uses bare `16`, `7`, `0x20..0x7f`; three loops with the line width repeated | `src/bin/qcow2_tool.rs:114-135` | Magic numbers |
| L2 | `if (l1_index as u64) >= l1.len() as u64` — casts both sides to compare a bound; `l1_index as usize >= l1.len()` says it | `src/reader.rs:983` | Dense expression |
| L3 | `cmd_info` hand-pads 15 label strings to column 19 | `src/bin/qcow2_tool.rs:54-78` | Cosmetic duplication |
| L4 | `open_best_effort` discards the RW error (`Err(_) =>`), so "fell back because read-only" is indistinguishable from "image is broken" | `src/reader.rs:221-227` | Swallowed context |
| L5 | `docs/future.md` cites `src/reader.rs:866`, `:885`, `:939-960` as the extended-L2 / decode sites; all three have drifted (866 is now `allocate_l2_table`) | `docs/future.md` | Comments that lie |
| L6 | `capi.rs` tests re-implement `build_image` + `tmp_path` that already exist in `tests/common/mod.rs` (unit/integration boundary makes sharing awkward — see *Considered and not flagged*) | `src/capi.rs:154-206` | Duplicated code |
| L7 | Five test helpers each re-open the fixture with `OpenOptions` + seek + read/write: `bump_refcount`, `clear_copied_l2_entry_0`, `read_l2_entry_0`, `read_refcount_entry`, `patch_be` | `tests/synthetic.rs:589-653`, `894-902` | Duplicated code |

---

## What to fix first

1. **H1** — it is the only finding that loses user data. Fix order: add a
   failing test first (child image with a backing parent, write 4 bytes into an
   unallocated cluster, assert the rest of the cluster still reads the parent's
   bytes), then split `ClusterMap::Zero | ClusterMap::Unallocated` into two arms
   and give the unallocated arm a backing copy-up. `qemu-img check` on the
   result via the existing `qemu-validation` harness is the confirmation.
2. **H5** — introduce `ClusterAddress` / `split_virtual()` and one
   `repoint_l2(address, host_off)` helper. Doing this *before* H2/M3/M4 makes
   those refactors mechanical, and it is the change that stops an H1-shaped bug
   recurring: the three write branches become one shape with one differing
   seed step.
3. **H3 + H2 + M10** — pure documentation, zero regression risk, and they are
   actively misleading a downstream FFI consumer today. Cheapest correctness
   win in the report.
4. **H4 + M8** — name the descriptor fields, export the offset/entry helpers,
   and have the fixture builders *encode* through the same helpers the reader
   *decodes* with. Add one compressed fixture at a non-sector-aligned host
   offset while you are there; that arm of the span calculation has never run.
5. **M1 + M2** — collapse the three refcount preambles and fold
   `read_refcount`/`decrement_refcount` onto a shared
   `with_refcount_entry(host_off, f)`; the deliberate difference in their
   "block absent" handling becomes visible instead of buried.
6. Everything else is comprehension tax, not risk. M3/M4 (god functions) are
   large diffs and both have solid test cover, so they are safe but should
   follow the H5 rename so they are not done twice.

---

## Considered and not flagged

| Item | Reason |
|---|---|
| `Header::parse` returning a 19-field struct positionally | *Acceptable pattern* — it is a flat on-disk record; a builder would add indirection without adding clarity |
| `read_u32` / `read_u64` free functions in `header.rs` | *Acceptable pattern* — two lines each, correctly named, idiomatic |
| `capi.rs` `catch_unwind` + null checks at the FFI boundary | *False positive* for "defensive code for scenarios that can't happen" — C callers genuinely can pass NULL, and unwinding across the ABI is UB |
| `error.rs` one-variant-per-failure-shape | *Already good* — the module doc states the intent and the code matches it |
| L6 (`capi.rs` test fixture duplication) raised only as Low | *Below threshold* — Rust's unit/integration split makes sharing a fixture builder across `src/` and `tests/` genuinely awkward; only 2 instances |
| `ExtentIter` doing a per-cluster lookup rather than an L2-block scan | *Out of scope* — that is a performance design question, not readability, and the doc comment is honest about what it does |
| `decode_compressed_descriptor`'s arithmetic | Verified **correct** against the spec formula; flagged for naming (H4), not for behaviour |

---

## Test results

No code was changed, so before and after are identical. Recorded here as the
baseline any Phase 2 work must hold.

| | Before | After |
|---|---|---|
| Tests passing (`cargo test --locked`) | 53 (23 lib + 30 integration) | 53 — unchanged, nothing was modified |
| Tests failing | 0 | 0 |
| Gated tests (`--features qemu-validation`) | 15 (not run in this scan) | 15 |
| `cargo clippy --locked --all-targets` | clean | clean |
| Coverage | not instrumented (no coverage tool configured in this crate) | unchanged |

Coverage gaps identified during triage, all of which want a test *before* the
corresponding fix lands:

- write into an unallocated cluster on an image **with** a backing parent (H1)
- compressed cluster at a non-sector-aligned host offset (H4)
- compressed cluster at a cluster size other than 4 KiB (H4)
- `allocate_cluster` against an image whose refcount block leaves cluster 0 free (M12)
- explicit L2-cache invalidation after `update_l2_entry` (M13)
