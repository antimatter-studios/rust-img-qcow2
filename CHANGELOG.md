# Changelog

Notable changes to `am-img-qcow2`, newest first. This is a `0.x` crate, so the
**minor** is the compatibility boundary: a minor bump may break API, a patch
never does.


## [Unreleased]

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
