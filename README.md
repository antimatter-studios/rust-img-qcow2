# qcow2

Pure-Rust reader and writer for the QCOW2 disk-image format. Spec-derived;
no GPL code is copied or linked. Exposes a Rust API and a C ABI suitable
for FFI from C/C++/Go/Swift.

## Status

Read path: complete for the common case. Write path: phases A/B/C landed
(allocated / sparse / compressed clusters); phase D (refcount-block growth
and snapshot CoW) is the remaining gap.

### Read

- [x] Header parse (v2 and v3)
- [x] L1 / L2 cluster lookup, uncompressed clusters
- [x] Sparse / all-zero clusters (v3 zero flag honoured)
- [x] zlib-compressed clusters
- [x] Backing-file chain (parent path resolution + fall-through reads)
- [ ] zstd-compressed clusters
- [ ] Internal snapshots (image opens read-only when `nb_snapshots > 0`)

### Write

- [x] Phase A — write into already-allocated clusters
- [x] Phase B — sparse-grow (allocate cluster + L2 entry + refcount,
      crash-safe ordering)
- [x] Phase C — compressed-cluster rewrite (decompress → modify →
      reallocate → update L2)
- [x] `decrement_refcount` on cluster replacement (no longer leaks the
      old compressed cluster after rewrite)
- [ ] Phase D — refcount-block growth (currently returns `Unsupported`
      when an existing refcount block is full)
- [ ] Phase D — snapshot copy-on-write (writing a shared cluster while
      `nb_snapshots > 0`)

## API surface

```rust
use qcow2::Qcow2Reader;

// Read-only
let r = Qcow2Reader::open("disk.qcow2")?;
println!("virtual size: {}", r.virtual_size());

let mut buf = vec![0u8; 4096];
r.read_at(0, &mut buf)?;

// Read/write
let rw = Qcow2Reader::open_rw("disk.qcow2")?;
rw.write_at(0, b"hello")?;
rw.flush()?;
```

Headline methods on `Qcow2Reader`: `open`, `open_rw`, `open_best_effort`,
`virtual_size`, `cluster_size`, `version`, `header`, `has_backing`,
`is_writable`, `read_at`, `write_at`, `flush`.

## Layout

```
src/
  lib.rs       public API: Qcow2Reader
  error.rs     Error / Result
  header.rs    on-disk header parser (v2 + v3)
  reader.rs    L1/L2 lookup, refcount, decompress, read_at/write_at/flush
  capi.rs      C ABI
  bin/
    qcow2_tool.rs   CLI: info, read, dump
tests/
  synthetic.rs      hand-build minimal qcow2 in test, round-trip via API
```

## CLI

```
qcow2_tool info  <file>            # header + geometry
qcow2_tool read  <file> <off> <len>  # read len bytes at virtual offset off
```

## Spec

QEMU's [qcow2 specification][spec] is the reference. This crate implements
the on-disk format from the public spec — it does not vendor or link any
code from the QEMU project.

[spec]: https://github.com/qemu/qemu/blob/master/docs/interop/qcow2.txt

## License

MIT.
