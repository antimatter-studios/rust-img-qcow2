# Features

What this crate does today, what it refuses, and what is coming. **Every
pull request that adds, fixes, refuses or removes behaviour updates its row
here, in the same pull request** (AGENTS.md). The reasoning behind each change
is in [CHANGELOG.md](../CHANGELOG.md); the plan for the formats it still
refuses is in [future.md](future.md).

**Since** is the release a row's current state shipped in, with the issue or
pull request the changelog cites for it. Work merged after the last release
is **Unreleased (#N)** until the next one. **Tracking** names the issue, or
the section of `future.md`, for anything not finished.

States:

- **Supported**: works, and is checked against `qemu-img` (`check`, `info`,
  `convert`).
- **Experimental**: works in every test, but is new.
- **Partial**: works for part of the case, and the row says which part.
- **Refused**: recognised and refused by name, rather than misread.
- **Not supported**: neither read nor refused by name.
- **Upcoming**: an open issue with a plan.

## Reading

| Feature | State | Since | Tracking | Checked by |
|---|---|---|---|---|
| Header, version 2 and 3 | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| L1 / L2 lookup, uncompressed clusters | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| Unallocated and zero-flagged clusters read as zeros | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| zlib-compressed clusters | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| zstd-compressed clusters (`compression_type = 1`) | Supported | 0.2.0; checked against `qemu-img` 0.3.2 | | `synthetic.rs`, `qemu_validation.rs` |
| Backing-file chains, with a depth limit | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| Allocation map (`extents`, `cluster_status_at`) | Supported | 0.3.1 | | `synthetic.rs`, `qemu_validation.rs` |
| A compressed descriptor or L2 entry that points outside the image, or is unaligned | Refused | 0.5.0 | | `synthetic.rs`, `qemu_validation.rs` |
| A backing name running past the first cluster | Refused | 0.5.0 | | `qemu_validation.rs` |
| Opening a damaged image as far as it reads (`open_best_effort`) | Supported | 0.2.0 | | |
| Encryption (AES or LUKS) | Refused | 0.2.0 | future.md | `src/header.rs` unit tests |
| External data file | Refused | 0.2.0 | future.md | `src/header.rs` unit tests |
| Extended L2 entries | Refused | 0.2.0 | future.md | `src/header.rs` unit tests |
| Any other unknown incompatible bit or compression type | Refused | 0.2.0 | | `src/header.rs` unit tests |
| Reading an internal snapshot's own view | Not supported | | | |
| Fuzzed header and mapping parsers | Supported | 0.5.0 | | `fuzz_decoders.rs` |

## Writing

| Feature | State | Since | Tracking | Checked by |
|---|---|---|---|---|
| Write into allocated clusters | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| Allocating writes: a new cluster, L2 entry and refcount, in a crash-safe order | Supported | 0.2.0 | | `synthetic.rs`, `qemu_validation.rs` |
| Refcount-block growth when every block is full | Supported | 0.2.0 | | `synthetic.rs` |
| Rewriting a compressed cluster: decompressed, changed, reallocated | Supported | 0.2.0 | | `synthetic.rs` |
| A replaced cluster's refcount decremented | Supported | 0.2.0 | | `synthetic.rs` |
| Copy-on-write of a cluster an internal snapshot shares (refcount above 1) | Supported | 0.2.0 | | `synthetic.rs` |
| An allocating write on a backed image copies the backing data up | Supported | 0.4.3 | | `synthetic.rs` |
| Concurrent writers | Supported | 0.5.0 | | `qemu_validation.rs` |
| Writing an image flagged corrupt or with dirty refcounts | Refused | 0.5.0 | | `src/header.rs` unit tests |
| Repairing refcounts | Not supported | | | |
| Creating an image | Not supported | | | `tests/cli/test-unsupported.sh` |
| Resizing an image | Not supported | | | `tests/cli/test-unsupported.sh` |
| Writing compressed clusters: a write to a compressed cluster stores it uncompressed | Not supported | | | |

## Interfaces

| Feature | State | Since | Tracking | Checked by |
|---|---|---|---|---|
| Rust API (`Qcow2Reader`), over a path or an `rust-fs-core` device | Supported | 0.2.0 | | `synthetic.rs` |
| C ABI | Supported | 0.2.0 | | `src/capi.rs` unit tests, `header_names_the_built_library.rs` |
| `img.qcow2` `info`/`get`, `read`, `write` (`--features cli`) | Supported | 0.5.0 | | `cli_write.rs`, `tests/cli/test-info.sh`, `tests/cli/test-read.sh`, `tests/cli/test-write.sh` |
| `img.qcow2` `create`, `resize`, `set` | Not supported (`not implemented`, exit 3) | 0.5.0 | | `tests/cli/test-unsupported.sh` |
| `rust-img-qcow2 doctor`, man pages, shell completions | Supported | 0.5.0 | | `tests/cli/test-names.sh`, `tests/cli/test-docs.sh` |
