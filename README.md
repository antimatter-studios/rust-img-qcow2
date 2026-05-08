# qcow2

Pure-Rust reader for the QCOW2 disk-image format. Spec-derived; no GPL code is
copied or linked. Exposes a Rust API and a C ABI suitable for FFI from
C/C++/Go/Swift.

## Status

Phase 1 — read-only.

- [x] Header parse (v2 and v3)
- [x] L1 / L2 cluster lookup, uncompressed clusters
- [x] Sparse / all-zero clusters
- [ ] zlib / zstd compressed clusters
- [ ] Backing-file chain
- [ ] Internal snapshots
- [ ] Write support (refcount table updates, cluster allocation)

## Layout

```
src/
  lib.rs       public API: Qcow2Reader
  error.rs     Error / Result
  header.rs    on-disk header parser (v2 + v3)
  reader.rs    L1/L2 lookup + read_at
  bin/
    qcow2_tool.rs   CLI: info, read, dump
tests/
  synthetic.rs      hand-build minimal qcow2 in test, round-trip via API
```

## Usage

```rust
use qcow2::Qcow2Reader;

let r = Qcow2Reader::open("disk.qcow2")?;
println!("virtual size: {}", r.virtual_size());

let mut buf = vec![0u8; 4096];
r.read_at(0, &mut buf)?;
```

## CLI

```
qcow2_tool info  <file>            # header + geometry
qcow2_tool read  <file> <off> <len>  # read len bytes at virtual offset off
```

## Spec

QEMU's [qcow2 specification][spec] is the reference. This crate implements the
on-disk format from the public spec — it does not vendor or link any code from
the QEMU project.

[spec]: https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.txt
