# Changelog

Notable changes to `am-img-qcow2`, newest first. This is a `0.x` crate, so the
**minor** is the compatibility boundary: a minor bump may break API, a patch
never does.


## [Unreleased]

### Fixed

- **Allocating a cluster asks the device for room instead of writing past its
  end.** `write_at` used to extend a `FileDevice` implicitly, and that is how
  this format allocated: append the cluster, then record where it went.
  `am-fs-core` withdrew that in its #75 — correctly, because the file grew
  while `size_bytes()` went on reporting the length taken at open, so a
  caching device could hold bytes no bounded read could reach
  (rust-fs-core#70). Three tests in `tests/synthetic.rs` failed against any
  core past `v0.2.10`, each one a write landing exactly at the device's end.

  `Qcow2Reader::dev_grow_for_cluster` is the replacement, called at both of
  `allocate_cluster`'s return paths — pass 1 hands out a free cluster from an
  existing refcount block, pass 2 creates a new block and hands out the
  cluster after it, and one call there covers the block in front of it. It
  **only ever grows**: `set_len` *sets*, so a smaller length would truncate,
  and a device already long enough is left alone — which keeps a writable but
  fixed-length device working for every allocation that fits inside it.

  It is deliberately **not** in `dev_write`: that funnel is also the in-place
  and metadata path, and growing there would extend the image on any stray
  offset, dissolving the bounds check rust-fs-core#70 exists to provide.

### Changed

- **`am-fs-core` moves to v0.2.13, and CI checks core out once instead of
  five times.** The dependency was held at `v0.2.10` by the refusal above
  while `scripts/tier.sh` needed `v0.2.13` for the output-budget wrapper, so
  every job cloned core twice at two refs — four times in `ci.yml` plus
  `release.yml` and `fuzz.yml`. The higher pin satisfies both bounds, so
  `FS_CORE_ROOT` points at the sibling the crate compiles against and the
  tooling-only clones are gone.

- **`Qcow2Reader` as a `BlockDevice` states that it does not grow.** It
  answered `can_grow() == false` by inheriting the trait default; now it says
  so. The guest-visible length there is the header's `size` field, and
  changing that is resizing the virtual disk rather than letting the host file
  get longer — a defaulted method left unmentioned reads exactly like one
  nobody considered.

- **`CountingWrites` in the tests forwards `set_len` and `can_grow`.** Both
  are defaulted on `BlockDevice`, to `Err(ReadOnly)` and `false`, so a
  passthrough wrapper that omits them looks writable and refuses to grow.
  Every allocating test through it failed with a bare `ReadOnly` once the
  allocator started asking for room.

### Added

- **The header and mapping parsers are fuzzed, on two tiers.** Every field
  a qcow2 image controls is an offset or a shift used in arithmetic —
  `cluster_bits`, `l1_size`, `l1_table_offset`, `refcount_table_offset` —
  and none of it had a fuzz target. `fuzz/` holds `image` and `header`
  and runs nightly on a bounded budget; `tests/fuzz_decoders.rs` is the
  gate, replaying and mutating the same corpus deterministically on the
  stable toolchain in under a second.

  The corpus is five images `qemu-img` wrote: the default cluster size, a
  small one, a version 2 file, a compressed one, and one with a backing
  reference. `the_corpus_reads_back_what_qemu_img_wrote` checks that this
  crate returns the pattern `qemu-img` put there — `A` at 0, `B` at
  500,000, zeros in the hole between — so the reference implementation is
  the standard on every pull request, on a machine with no `qemu-img`
  installed. It also asserts that a backed image opened on a *device* is
  refused for the backing chain rather than silently returning the holes
  as zeros (#107).

## [0.4.5] — 2026-09-06

### Fixed

- A compressed cluster's decode is bounded, and so are the tables read
  at open. A qcow2 header states the size of its refcount and L1 tables
  and each compressed cluster states how much it decodes to; all three
  came off the image and none was checked against what the file can
  hold, so a crafted image could make the reader allocate on its say-so.

## [0.4.4] — 2026-09-04

### Fixed

- **A defect that could wipe the header**, found while acting on the review's
  High and Medium findings.

### Changed

- **The addressing has names.** Splitting a virtual offset into its L1/L2/
  cluster parts was open-coded at each use; it is now one operation, with the
  cluster-address type distinguished from a raw byte offset.
- **One refcount locator, with the divergence named rather than merged.** The
  refcount lookup existed in more than one form; the forms differed for a
  reason, so the shared part is now shared and the difference is stated
  instead of being papered over.
- Coverage added for the paths nothing was reaching — mutation testing showed
  seven mutations of the reader that no test noticed.

## [0.4.3] — 2026-08-29

### Fixed

- **The backing chain is copied up when a write allocates.** Allocating a
  cluster on write without first pulling the backing file's contents into it
  loses whatever the backing image held for the untouched part of that cluster.

### Added

- `chore` tasks own this crate's build, and the code-review report is recorded
  in the repo.
- The github-guard hook set replaces the hand-rolled pre-commit hooks.

## [0.4.2] — 2026-06-21

### Changed

- The publish job clones its path-dependency siblings, pinned to a tag rather
  than tracking a branch, and publishing is gated on the disk-image validator
  cross-check. A release built from a floating dependency is not reproducible.

## [0.4.1] — 2026-06-09

### Changed

- Pinned toolchain moves from 1.94.1 to 1.95.0, in lockstep with the rest of
  the family. A straggler links two copies of `_rust_eh_personality` into any
  consumer that binds both.

## [0.4.0] — 2026-06-01

### Added

- `open()` rejects encrypted images and images with an external data file,
  rather than reading them as if the bytes were there.
- Header parse and `check_supported` unit tests; zstd compression is
  cross-validated.

## [0.3.2] — 2026-05-19

### Added

- Cross-validation harness against an external disk-image validator, with the
  synthetic builders checked against it.

### Fixed

- Compressed L2 entries no longer carry the COPIED flag, which is meaningless
  for them.

## [0.3.1] — 2026-05-19

### Added

- **`allocated_extents`**, so a sparse-aware consumer can ask which ranges are
  actually backed instead of reading holes.
- Release-on-tag pipeline using trusted publishing.

## [0.2.0] — 2026-05-12

### Added

- Device-backed reader, and CI (test, fmt, clippy).

### Changed

- `am-fs-core` and `am-partitions` dependencies move to 0.2.

[Unreleased]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.4...HEAD
[0.4.4]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.2.0...v0.3.1
[0.2.0]: https://github.com/antimatter-studios/rust-img-qcow2/releases/tag/v0.2.0
