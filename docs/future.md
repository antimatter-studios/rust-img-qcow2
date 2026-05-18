# Plan: closing the remaining `Error::Unsupported` paths

Scope: four qcow2 spec features that the reader still refuses. Each section
covers (a) what the feature is, (b) what needs implementing, (c) what we
still need to research, (d) licence / IP exposure.

Status snapshot (against `main`, crate v0.2.0):

| Feature             | Header probe        | Current behaviour          | Status        |
|---------------------|---------------------|----------------------------|---------------|
| zstd clusters       | `compression_type=1`| **decoded** via `ruzstd`   | **done in v0.2.0** |
| encryption          | `crypt_method != 0` | `Unsupported("encryption (AES or LUKS)")` | not started |
| external data file  | incompat bit 2      | `Unsupported("external data file")`       | not started |
| extended L2         | incompat bit 4      | `Unsupported("extended L2 entries")`      | not started |

Reject sites: `src/header.rs:182` (`check_supported`). Cluster decode site
that will branch on extended-L2: `src/reader.rs:866`.

---

## Cross-cutting: licence / IP discipline

Project is MIT. Two hard rules:

1. **No QEMU source.** QEMU is GPLv2. Do not read `block/qcow2*.c`,
   `block/crypto.c`, or any QEMU tree file as a reference while
   implementing. Spec PDFs, `docs/interop/qcow2.txt` (spec, not code),
   LUKS2 on-disk spec, RFCs — fine.
2. **No GPL/LGPL runtime deps.** Allowed: MIT, Apache-2.0, BSD,
   ISC, Unicode. Refuse: GPL, LGPL (static-link concern for the
   `staticlib` crate-type), MPL when in doubt.

Tooling: run `cargo deny` (or equivalent) before each feature lands;
already have `code-audit` skill for periodic sweeps. Every new crate
needs a one-line justification in the PR.

Spec sources to anchor work (public, redistributable text):

- qcow2 spec: <https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.txt>
  (the spec file itself; treat the surrounding C as off-limits).
- LUKS2 on-disk format: <https://gitlab.com/cryptsetup/cryptsetup/-/wikis/LUKS2-docs>
  (Apache-2.0 / CC-BY-SA spec doc — verify licence before pulling
  excerpts into our repo).
- Zstandard: RFC 8878.

If we have to quote spec language into source comments, paraphrase
rather than copy verbatim to keep distance from the upstream licence.

---

## 1. zstd clusters — done, documenting for reference

Spec: qcow2 v3 adds `compression_type` field at header byte 104; bit 3
of `incompatible_features` (`COMPRESSION_TYPE`) must be set when value
is non-zero. Value 1 = zstd.

What landed (v0.2.0):

- `src/header.rs:73` parses `compression_type`.
- `src/reader.rs:939–960` branches on it: deflate via `flate2`, zstd
  via `ruzstd` (streaming decoder, output capped at cluster_size).
- Synthetic fixture in `tests/synthetic.rs` uses `zstd` (dev-dep) to
  build a real zstd cluster, round-trips through the reader.

Licences: `ruzstd` (MIT), `flate2` (MIT/Apache-2.0), `zstd` dev-dep
only (BSD-3 / GPLv2 dual via zstd-sys — fine as a dev-dep, must not
escape into `[dependencies]`).

Research still open:

- Decompressed-length sanity: ruzstd doesn't expose the frame's
  declared content size cheaply; we currently cap by cluster_size.
  Confirm qemu always emits a single frame per cluster with
  content-size present; if so add an assertion.

---

## 2. Encryption — AES (legacy) + LUKS-in-qcow2

This is the biggest piece. Two distinct on-disk formats hide behind
`crypt_method`:

- `crypt_method = 1` → legacy AES-128-CBC, per-sector. Deprecated by
  upstream since ~2017. Read-only support is sufficient.
- `crypt_method = 2` → LUKS2 header embedded in the qcow2 file. This
  is the modern path and the one users actually have.

### 2a. Legacy AES (crypt_method = 1)

Per spec: 16-byte AES-128 key derived from password via 16 rounds of
something (need to confirm — spec says "Use AES with 128-bit key and
sector-based ESSIV-like IV"). Each 512-byte logical sector is
independently CBC-encrypted; IV is derived from the *guest* sector
number.

Implementation sketch:

- Add `aes = "0.8"` + `cbc = "0.1"` (RustCrypto, MIT/Apache).
- Plumb a `key: Option<[u8; 16]>` into `Qcow2Reader`; new constructor
  `open_encrypted(path, password)`.
- Decrypt inside `read_at` *after* the cluster bytes are fetched but
  *before* they're returned. Iterate per 512-byte sector, derive IV
  from `(guest_offset / 512)`, CBC-decrypt in place.

Research needed:

- Exact key-derivation: the spec text is thin. Verify against a
  fixture image we build with `qemu-img create -o
  encryption=on,encrypt.format=aes ...` and check key bytes via
  `qemu-img info --object`. Do **not** read `block/crypto.c`.
- Exact IV scheme. Spec says "plain64" or similar — confirm from
  a fixture, not from QEMU source.

Risks: legacy AES is deprecated upstream; we may decide to refuse it
permanently and only implement LUKS. That's a product call.

### 2b. LUKS-in-qcow2 (crypt_method = 2)

The qcow2 file embeds a full LUKS2 header in a region pointed at by a
header-extension entry. Once the LUKS volume key is unwrapped, each
512-byte sector of the qcow2 *payload* (cluster contents) is
en/decrypted using the LUKS cipher (typically AES-XTS-plain64,
512-bit key → two 256-bit halves).

Pieces required:

1. **Header-extension parser** for the LUKS metadata region. qcow2
   header extensions live between the fixed header and the L1 table.
   Format: `(u32 type, u32 length, payload, pad to 8)`. The
   crypto-header-extension type number is in the spec; record it.
2. **LUKS2 binary header parse**: JSON metadata area (LUKS2 stores
   keyslots, segments, digests as JSON inside the binary header).
   Need a `serde_json` dep (MIT/Apache) or hand-roll a minimal
   parser.
3. **Key unwrap**: PBKDF2 *or* Argon2id (LUKS2 keyslots usually
   Argon2id). Crates: `pbkdf2`, `argon2`, `sha2`, `hmac` — all
   RustCrypto, MIT/Apache.
4. **Anti-forensic splitter (AFsplit)** to recover the keyslot key
   material. Algorithm is spec-defined (Fruhwirth, 2004). Not in
   RustCrypto AFAIK — implement from the LUKS spec.
5. **Sector cipher**: AES-XTS-plain64. Crate `xts-mode` or
   `aes` + manual XTS. Verify licence.
6. **Plumb the password** through the API surface and the C ABI.

Research needed:

- Header-extension type number for the LUKS region (spec lists it).
- LUKS2 sector size in qcow2 context — is it always 512, or does it
  follow the qcow2 cluster?
- Whether LUKS1 also needs supporting (older images) or whether v2
  is enough.
- Argon2id parameters: read from the JSON metadata, not hard-coded.
- API ergonomics: do we take `Vec<u8>` keys, a passphrase, or a
  callback? Probably callback for tooling that wants to prompt.

Licence exposure: every crypto crate proposed is RustCrypto
(MIT/Apache-2.0 dual). `serde_json` is MIT/Apache. No GPL contact
as long as we don't pull `cryptsetup`.

Test strategy: build fixture qcow2s with `qemu-img` once, commit the
ciphertext + the known password, decrypt in tests. Do not depend on
having `qemu-img` at test time — pre-build and check in the fixtures.

Effort estimate: this is a two-to-three week feature on its own.
Recommend separating into PRs: (i) header-extension parser, (ii)
LUKS2 JSON metadata, (iii) keyslot unwrap, (iv) sector decrypt
hooked into `read_at`.

---

## 3. External data file (incompat bit 2)

Layout: the qcow2 holds L1, L2, refcount tables, header, snapshots —
but data clusters live in a separate raw file. L2 entries still
encode "host offset"; that offset now indexes into the external file
rather than the qcow2.

Implementation sketch:

- Parse the data-file path from its header extension (extension type
  number is in the spec; confirm).
- Resolve the path relative to the qcow2 file's parent dir (matches
  backing-file resolution we already do in `reader.rs`).
- Add a second `File` (or `BlockDevice`) handle inside `Qcow2Reader`
  for the external data file.
- In the cluster read path, when this bit is set, every `dev_read`
  for a data cluster goes to the external handle. Metadata reads
  (L1, L2, refcount) stay on the qcow2 handle.
- Write path: refcount tracking is *optional* for external-data
  images per spec (the qcow2 may not bother counting refs on the
  external file). Decide whether v1 of this feature is read-only.

Research needed:

- Header-extension type number for the data-file path.
- Whether the external file may itself be qcow2 (probably not — spec
  says "raw") or only `raw`.
- Interaction with `backing_file_offset` — can an external-data
  image also have a backing file? Spec answer, then test.
- Refcount semantics on writes: does qemu still increment refcounts
  in the metadata file when extending the external data file? If
  not, simplify our write path for this case.

Licence exposure: zero — this is plumbing.

Effort: small. ~1 week including fixture and tests, mostly the
extension-parsing and path-resolution code.

---

## 4. Extended L2 entries (incompat bit 4)

What changes: L2 entries grow from 64 bits to 128 bits. The cluster
is conceptually split into 32 subclusters; the extra 64 bits encode
per-subcluster *allocation* and *all-zero* bits (32 + 32). Lets the
image track sparseness at sub-cluster granularity, useful when
guest writes are smaller than `cluster_size`.

Concrete spec points:

- Entries-per-L2 changes: `cluster_size / 16` (not `/8`).
- Standard L2 layout (offset, COPIED, COMPRESSED, ZERO bits)
  stays in the first 8 bytes.
- Second 8 bytes: 32-bit "allocation bitmap", 32-bit "zero bitmap".
- Compressed clusters: spec says compression is **not** allowed
  with extended L2 — error if both flags are set. Need to enforce.

Implementation sketch:

- Header parse: branch on `EXTENDED_L2` bit; store `l2_entry_size`
  (8 or 16) in `Header`.
- `Header::l2_entries()` uses `cluster_size / l2_entry_size`.
- Touch every L2 access in `reader.rs` — they currently assume
  u64-per-entry. Most live around `read_l2_entry` at
  `src/reader.rs:885`. Refactor to return a richer
  `L2Entry { offset, flags, subcluster_alloc: u32, subcluster_zero: u32 }`.
- `cluster_status` (`src/reader.rs:866`) becomes subcluster-aware:
  for a read at virtual offset `v`, derive the subcluster index
  `(v % cluster_size) / (cluster_size / 32)` and consult the
  bitmaps. Zero subcluster → emit zeros without touching disk.
- Write path: writing into an unallocated subcluster of an
  *allocated* cluster sets the alloc bit, clears the zero bit, and
  bumps the cluster's refcount only if the host cluster wasn't
  already allocated. Lots of fiddly bit work — write a focused unit
  test for each transition.

Research needed:

- Exact bit ordering inside the two u32 bitmaps (LSB = subcluster 0
  or MSB = subcluster 0?). Spec is explicit; confirm.
- Whether L1 layout changes (it does not — L1 still points at L2
  tables, but each L2 table is now twice the size).
- Interaction with backing files: when a subcluster is unallocated
  *and* its alloc bit is clear, do we fall through to backing?
  (Yes, per spec, but verify with a fixture.)

Licence exposure: zero.

Effort: medium. ~1–1.5 weeks for read path; another similar slice
for write path. The refactor of `read_l2_entry` will be the biggest
diff — every existing test must keep passing.

---

## Suggested order of attack

1. **External data file** — smallest scope, exercises the
   header-extension parsing we'll also need for LUKS. Land first.
2. **Extended L2** — pure spec mechanics, no crypto, but touches the
   reader's hot path. Land second, separate PR.
3. **LUKS-in-qcow2** — biggest, most IP-sensitive. Build on the
   header-extension parser from step 1. Multiple PRs.
4. **Legacy AES** — only if real users surface; otherwise document
   as "won't fix, image deprecated upstream".

Each step gates the next via the `Error::Unsupported` site in
`src/header.rs:182`: drop the corresponding refusal only when the
read path is provably correct against committed fixture images.

---

## Open product questions

- Read-only vs read-write parity: for v1 of each feature, is
  read-only acceptable? Recommendation: yes, ship read first, write
  in a follow-up.
- Fixture provenance: we'll generate ciphertexts with `qemu-img` and
  check them in. That's data, not GPL code — fine. Note this
  explicitly in the test file headers so future contributors don't
  worry.
- C ABI: encryption needs a password input on `open`. Decide whether
  the C side takes a `const char*` password or a callback. Callback
  is friendlier for GUI clients.
