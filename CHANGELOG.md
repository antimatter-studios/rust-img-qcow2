# Changelog

Notable changes to `rust-img-qcow2` (published as `am-img-qcow2` until its last version), newest first. This is a `0.x` crate, so the
**minor** is the compatibility boundary: a minor bump may break API, a patch
never does.


## [0.6.0] — 2026-10-06

### Changed

- **Published as `rust-img-qcow2`, the repository's name.** The crate was `am-img-qcow2`
  until its last version, which stays on crates.io pointing here. A
  dependent changes one line in `Cargo.toml`; the import moves from `qcow2` to `img_qcow2`, and the C symbols are unchanged.
- **Depends on `rust-fs-core` 0.3.0**, the same library under its new name.

## [Unreleased]

### Changed

- **A release's notes are its CHANGELOG section.** The release workflow
  takes the GitHub release's body from `scripts/core.sh release-notes` and
  refuses a tag the CHANGELOG does not describe, before anything is
  published (rust-fs-core#209). It depends on rust-fs-core 0.3.1.

## [0.5.2] — 2026-10-06

### Renamed

- **The last version published as `am-img-qcow2`.** The crate is renamed to
  `rust-img-qcow2`, the repository's name; every later version is published under
  that name only, starting at 0.6.0. The description and the README say where
  the crate went. The import changes too: `use qcow2::...` becomes `use img_qcow2::...`.


### Changed

- **The release tarballs are packaged, attested and attached by
  rust-fs-core's `release-cli` workflow, not by a copy here** (#146).
  `release.yml`'s `package-cli` and `release-cli` jobs become one `cli` job
  calling `antimatter-studios/rust-fs-core/.github/workflows/release-cli.yml`
  at v0.2.23, pinned by commit SHA; `scripts/package-cli.sh` and its test
  are gone, and `ci.yml` and `chore package:cli` package through
  `scripts/core.sh package-cli`. What ships is declared in `Cargo.toml`'s
  `[package.metadata.package-cli]` and the tarball's layout is unchanged.
  The attestations now name the shared workflow, so a tarball is verified
  with `--signer-workflow
  antimatter-studios/rust-fs-core/.github/workflows/release-cli.yml`.
  am-fs-core moves to v0.2.23, the first release that carries it.

## [0.5.1] — 2026-09-30

### Fixed

- **The release packages the command-line tool on macOS.** 0.5.0's
  `darwin-arm64` leg failed in `scripts/package-cli.sh`: macOS's `/bin/bash`
  is 3.2, which ends a `$( )` at the first `)` of a bare `case` pattern, so
  the check for members outside the install-prefix layout did not parse and
  0.5.0 has no tarballs or GitHub release. Every pattern in that
  substitution now opens with `(`, which every bash parses. 0.5.0 is on
  crates.io; the library is unchanged.

## [0.5.0] — 2026-09-30

### Fixed

- **The public API docs build, and CI runs rustdoc.** `RUSTDOCFLAGS="-D
  warnings" cargo doc --no-deps` failed with eight errors and `ci.yml` had no
  `cargo doc` step at all, so nothing had ever seen them (#105). Three shapes:

  - links to **private** items from public documentation — `ClusterMap`,
    `ClusterSeed`, `MAX_BACKING_DEPTH`, `Qcow2Reader::plan_write`. A public doc
    cannot link to a private item, so those stop being links;
  - links that resolve to **nothing** — `[read_at]` and `[virtual_size()]`.
    Both items are public, so these are now `Qcow2Reader::read_at` and
    `Qcow2Reader::virtual_size` rather than being unlinked: the parentheses
    were the whole problem in the second;
  - a **redundant explicit link target** in `src/capi.rs`, where the target
    was what the shortcut already resolved to.

  Fixing those surfaced seven more that had been behind them: `qcow2_tool`'s
  usage block writes `<file>`, `<offset>` and `<len>`, which rustdoc reads as
  unclosed HTML tags. It is a `text` fence now.

  A link that goes nowhere is worse than no link, because it reads as a promise
  that something is documented elsewhere. The `fmt` job runs rustdoc with
  `-D warnings` now — rustdoc's default is to warn and carry on, which is how
  this reached eight.

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

- **`tests/changelog.rs`: the changelog's shape, checked rather than
  remembered.** *(#120)* Ported from `rust-img-vhdx`, where it was written for
  rust-img-vhdx#63 — a required public field added to a `pub struct` after a
  release, with the pending release on course to be a patch. Six assertions:

  - `[package].version` equals the newest `## [x.y.z]` section;
  - a released section carrying a breaking note bumped the **minor**, this
    family's rule for a `0.x` crate;
  - each `## [...]` section uses a `### Heading` at most once;
  - every released section has a `[x.y.z]: <url>` definition;
  - the marker scan reads a break however it is spelled, and does not match
    near-misses;
  - the version parser reads a heading or skips it, never guesses.

  It found two defects here on its first run, both fixed in this change:
  `[Unreleased]` carried `### Fixed` twice — one per PR that added a section
  instead of adding to the one already there — and there was no `[0.4.5]` link
  definition, with `[Unreleased]` still comparing `v0.4.4...HEAD`, so that
  heading rendered as literal brackets.

  The sibling that first had this reached **seven** `### Fixed` headings in one
  section, and a review bot reported it five times before anyone acted
  (rust-img-vmdk#71).

  One assertion was **removed** in the port rather than carried over. The
  original demanded the changelog contain a real breaking marker, so that a
  scan matching nothing could not pass vacuously. That is wrong for a crate
  whose released history has broken nothing: this one has only added public
  methods since v0.4.5, and the port failed on that *control* rather than on
  the rule — which is the same "a check that cannot fail" defect the control
  existed to prevent, arrived at from the other side. Proving the scan works
  belongs in a test of the scan, which is what
  `a_break_is_recognised_however_it_is_spelled` does.


- **`fuzz/Cargo.toml` follows this crate's `am-fs-core` pin, and a test says
  so.** *(#118)* The fuzz crate is a separate package with its own
  manifest and lockfile, so nothing about bumping the parent's dependency
  pointed at the child's: this one required `0.2.10` while the crate required
  `0.2.13`, and `fuzz.yml` already checked core out at `v0.2.13`.

  It was green throughout, which is the problem. `version = "0.2.10"` is a caret
  requirement that `0.2.13` satisfies, the `path` source is what cargo actually
  uses, and `cargo fuzz run` is not passed `--locked`, so the stale
  `fuzz/Cargo.lock` was rewritten in place on every run. The day core reaches
  `0.3.0` the parent resolves and the fuzz crate does not — and that surfaces
  in a nightly cron, naming a version requirement rather than the bump behind
  it.

  `the_fuzz_crate_requires_the_same_core_as_this_one` in
  `tests/fuzz_decoders.rs` compares the two manifests' `version` fields. It
  refuses a bare `path` dependency too, since one passes every other check in
  that file while saying nothing about which core it is for. Both failure modes
  were confirmed to fail before the fix went in.

  All four image crates had drifted, in three different ways.

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

- **`img.qcow2`, the command-line tool**, one multi-call binary named
  `rust-img-qcow2` behind a new `cli` feature (clap, MIT/Apache-2.0), so the
  static library gains no dependency. `img.qcow2 <image> info`/`get [key]`
  reports the image as JSON (`--text` for people), and `read [--offset N]
  [--length N] [-o FILE]` streams the guest's raw bytes, backing chain
  resolved and compressed clusters inflated, the whole virtual disk when no
  range is given. `write --offset N` writes stdin into the guest, refusing
  input that would run past the end before writing any of it; an image
  flagged dirty or corrupt answers `not implemented`. `create` (no creator
  in the library), `resize` and `set` answer `not implemented` (exit 3). `rust-img-qcow2 doctor` checks that the
  `img.qcow2` on `PATH` is this one. `chore test:cli` tests the installed tool
  against `qemu-img`, and CI runs it on every pull request. Man pages (section
  1, one per name and per subcommand) and zsh, bash and fish completions are
  generated by the binary itself (`rust-img-qcow2 generate man|completions
  SHARE`, with clap_mangen and clap_complete, MIT/Apache-2.0), so they cannot
  describe a flag it does not take.

- Releases carry a build-provenance attestation: the published `.crate` is
  attached to the GitHub release for its tag, checked first against the
  crates.io checksum, and verifiable with `gh attestation verify` (see the
  README, "Verifying a release").
- Releases attach the command-line tool as a tarball per platform
  (`darwin-arm64`, `linux-x86_64`), laid out as an install prefix
  (`bin/rust-img-qcow2`, `bin/img.qcow2` linked to it, man pages and
  completions under `share/`, `share/rust-img-qcow2/CAVEATS`, `LICENSE`) and
  attested with build provenance like the `.crate`. CI builds the tarball and
  checks its layout on every pull request.
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

### Removed

- **`qcow2_tool` is removed; `img.qcow2` replaces it** *(BREAKING for `cargo
  install --bin qcow2_tool`)*. `info` is `img.qcow2 <image> info`; `dump
  <file> <offset> <len>` is `img.qcow2 <image> read --offset N --length N`;
  the hex-dump `read` is that piped through `xxd`. It was never in a release
  tarball or formula.

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

[Unreleased]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.5.2...v0.6.0
[0.5.2]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.5...v0.5.0
[0.4.5]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.4...v0.4.5
[0.4.4]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/antimatter-studios/rust-img-qcow2/compare/v0.2.0...v0.3.1
[0.2.0]: https://github.com/antimatter-studios/rust-img-qcow2/releases/tag/v0.2.0
