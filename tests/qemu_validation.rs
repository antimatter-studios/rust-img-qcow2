//! Cross-validation against `qemu-img`.
//!
//! Gated behind the `qemu-validation` feature so regular `cargo test`
//! does not require qemu-img on PATH. Run with:
//!
//!     cargo test --features qemu-validation --test qemu_validation
//!
//! Licensing posture: `qemu-img` is invoked as a separate OS process.
//! No QEMU source or binary is linked into this crate, and `qemu-img`
//! is never bundled into a release artifact. Reading bytes that a GPL
//! tool happens to produce, or feeding it bytes for validation, does
//! not create a derivative work.

#![cfg(feature = "qemu-validation")]

mod common;

use common::*;
use qcow2::Qcow2Reader;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

const QEMU_IMG: &str = "qemu-img";

fn run_qemu(args: &[&str]) -> std::process::Output {
    Command::new(QEMU_IMG)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to invoke `{QEMU_IMG}` ({e}); install qemu-utils?"))
}

fn assert_qemu(args: &[&str]) {
    let out = run_qemu(args);
    assert!(
        out.status.success(),
        "`qemu-img {}` failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn qemu_create(path: &Path, size: &str) {
    assert_qemu(&["create", "-f", "qcow2", path.to_str().unwrap(), size]);
}

fn qemu_check(path: &Path) {
    assert_qemu(&["check", path.to_str().unwrap()]);
}

fn qemu_convert_to_raw(qcow: &Path, raw: &Path) {
    assert_qemu(&[
        "convert",
        "-f",
        "qcow2",
        "-O",
        "raw",
        qcow.to_str().unwrap(),
        raw.to_str().unwrap(),
    ]);
}

fn qemu_convert_raw_to_qcow2(raw: &Path, qcow: &Path) {
    assert_qemu(&[
        "convert",
        "-f",
        "raw",
        "-O",
        "qcow2",
        raw.to_str().unwrap(),
        qcow.to_str().unwrap(),
    ]);
}

/// Sanity: qemu-img is reachable and behaves as expected. If this
/// fails, every other test in this file would also fail uselessly.
#[test]
fn qemu_img_is_callable() {
    let out = run_qemu(&["--version"]);
    assert!(
        out.status.success(),
        "qemu-img --version exited non-zero — qemu-utils not installed?"
    );
}

/// Direction 1: structural validation. Build a qcow2 with qemu-img,
/// then `qemu-img check` it. Establishes that qemu-img's own output
/// passes its own validator on this host.
#[test]
fn qemu_check_passes_on_empty_qemu_image() {
    let p = tmp_path("empty");
    qemu_create(&p, "4M");
    qemu_check(&p);
}

/// Direction 2 (cross-read, trivial): a blank qcow2 from qemu-img is
/// all zeros to a reader. Catches any header field we mis-parse from
/// a real qemu-emit, since misparsing the header would corrupt the
/// L1/L2 lookup and produce non-zero garbage instead.
#[test]
fn our_reader_returns_zeros_for_empty_qemu_image() {
    let p = tmp_path("zeros");
    qemu_create(&p, "1M");

    let r = Qcow2Reader::open(&p).unwrap();
    let mut buf = vec![0u8; 65_536];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0),
        "expected all-zero read from empty qemu image"
    );
}

/// Direction 2 (cross-read, populated): convert a raw file with a
/// known byte pattern into qcow2 via qemu-img, then read it back with
/// our reader and compare. Validates our L1/L2/data-cluster decode
/// against a real qemu-produced layout, not just our own synthetic
/// builder.
#[test]
fn our_reader_returns_qemu_populated_pattern() {
    let raw = tmp_path("pattern-src");
    let qcow = tmp_path("pattern-dst");

    let mut data = vec![0u8; 4096 * 8];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    std::fs::write(&raw, &data).unwrap();
    qemu_convert_raw_to_qcow2(&raw, &qcow);

    let r = Qcow2Reader::open(&qcow).unwrap();
    let mut buf = vec![0u8; data.len()];
    r.read_at(0, &mut buf).unwrap();
    assert_eq!(buf, data, "byte mismatch reading qemu-produced image");
}

/// Direction 3 (cross-write, structural): create with qemu-img, write
/// with our writer, then `qemu-img check` validates structural
/// invariants — refcount consistency, L1/L2 reachability, no orphans.
/// Catches refcount-tracking bugs that a self-consistent reader could
/// not detect on its own.
#[test]
fn qemu_check_passes_on_image_we_wrote_to() {
    let p = tmp_path("we-wrote-check");
    qemu_create(&p, "1M");

    let r = Qcow2Reader::open_rw(&p).unwrap();
    r.write_at(0, b"qcow2 written by our crate").unwrap();
    r.flush().unwrap();
    drop(r);

    qemu_check(&p);
}

/// Direction 3 (cross-write, content): write bytes via our crate,
/// have qemu-img convert the resulting qcow2 back to raw, and verify
/// the bytes survived the round-trip. This is the strongest single
/// check — it would fail if our writer produced spec-valid-looking
/// bytes that qemu nonetheless interprets differently from us.
#[test]
fn qemu_can_extract_our_written_bytes() {
    let qcow = tmp_path("we-wrote-convert");
    let raw = tmp_path("we-wrote-convert-raw");
    qemu_create(&qcow, "1M");

    let payload = b"bytes-qemu-must-see-back-XYZ";
    let r = Qcow2Reader::open_rw(&qcow).unwrap();
    r.write_at(0, payload).unwrap();
    r.flush().unwrap();
    drop(r);

    qemu_convert_to_raw(&qcow, &raw);

    let out = std::fs::read(&raw).unwrap();
    assert_eq!(&out[..payload.len()], payload);
    assert!(
        out[payload.len()..].iter().all(|&b| b == 0),
        "rest of the converted raw image should be zero"
    );
}

// ---------------------------------------------------------------------------
// Direction 4: synthetic-builder validation. Each `build_*` in
// `tests/common/mod.rs` writes a hand-crafted qcow2 by hand. Without
// external validation our reader is the only judge of whether those
// bytes are spec-valid — that's the "marking our own homework" risk.
//
// Each test here builds a fixture with our own code, then asks qemu-img
// two questions:
//
//   1. `qemu-img check`  — are the bytes structurally consistent
//      (header, L1/L2 reachable, refcount block sums match)?
//   2. `qemu-img info --output=json` — does qemu parse the same fields
//      we encoded (virtual_size, cluster_size, refcount_bits, compat,
//      compression type, backing chain)?
// ---------------------------------------------------------------------------

fn qemu_info_json(path: &Path) -> Value {
    let out = run_qemu(&["info", "--output=json", path.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "qemu-img info failed:\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("qemu-img info JSON must parse")
}

/// Look up `format-specific.data.<key>` in qemu-img info JSON.
fn qcow_meta<'a>(info: &'a Value, key: &str) -> &'a Value {
    info.get("format-specific")
        .and_then(|f| f.get("data"))
        .and_then(|d| d.get(key))
        .unwrap_or_else(|| panic!("qemu-img info JSON missing format-specific.data.{key}"))
}

#[test]
fn qemu_check_passes_on_our_standard_synthetic() {
    let p = tmp_path("synth-standard");
    build_image(&p);
    qemu_check(&p);
}

#[test]
fn qemu_info_matches_our_standard_synthetic() {
    let p = tmp_path("synth-info");
    build_image(&p);

    let info = qemu_info_json(&p);
    assert_eq!(info["format"], "qcow2");
    assert_eq!(info["virtual-size"], VIRT_SIZE);
    assert_eq!(info["cluster-size"], CLUSTER_SIZE);
    // We encoded refcount_order = 4 → refcount_bits = 16. v3 image.
    assert_eq!(qcow_meta(&info, "compat"), "1.1");
    assert_eq!(qcow_meta(&info, "refcount-bits"), 16);
    // No compression-type field present means default zlib, OR the field
    // is "zlib". Accept either.
    if let Some(ct) = info["format-specific"]["data"].get("compression-type") {
        assert_eq!(ct, "zlib");
    }
    // No encryption, no backing file in build_image.
    assert!(info.get("backing-filename").is_none());
}

#[test]
fn qemu_check_passes_on_our_zlib_compressed_synthetic() {
    let p = tmp_path("synth-zlib");
    build_compressed_image(&p, 0xCC);
    qemu_check(&p);
}

#[test]
fn qemu_can_decompress_our_zlib_synthetic() {
    let qcow = tmp_path("synth-zlib-conv-q");
    let raw = tmp_path("synth-zlib-conv-r");
    build_compressed_image(&qcow, 0xCC);

    // qemu-img must decompress our compressed cluster identically.
    qemu_convert_to_raw(&qcow, &raw);
    let out = std::fs::read(&raw).unwrap();
    assert!(
        out[..CLUSTER_SIZE as usize].iter().all(|&b| b == 0xCC),
        "qemu's view of our compressed cluster diverges from ours"
    );
}

#[test]
fn qemu_check_passes_on_our_zstd_compressed_synthetic() {
    let p = tmp_path("synth-zstd");
    build_zstd_compressed_image(&p, 0xCC);
    qemu_check(&p);
}

#[test]
fn qemu_info_reports_zstd_for_our_zstd_synthetic() {
    let p = tmp_path("synth-zstd-info");
    build_zstd_compressed_image(&p, 0xCC);

    let info = qemu_info_json(&p);
    assert_eq!(qcow_meta(&info, "compression-type"), "zstd");
}

#[test]
fn qemu_can_decompress_our_zstd_synthetic() {
    let qcow = tmp_path("synth-zstd-conv-q");
    let raw = tmp_path("synth-zstd-conv-r");
    build_zstd_compressed_image(&qcow, 0xCC);

    qemu_convert_to_raw(&qcow, &raw);
    let out = std::fs::read(&raw).unwrap();
    assert!(
        out[..CLUSTER_SIZE as usize].iter().all(|&b| b == 0xCC),
        "qemu's view of our zstd cluster diverges from ours"
    );
}

#[test]
fn qemu_check_passes_on_our_backing_chain_pair() {
    let (parent, child, rel) = pair_paths("synth-backing-check");
    build_image(&parent);
    build_child_with_backing(&child, &rel, &[]);

    // Both files must independently pass qemu-img check.
    qemu_check(&parent);
    qemu_check(&child);
}

#[test]
fn qemu_info_reports_backing_path_on_our_child() {
    let (parent, child, rel) = pair_paths("synth-backing-info");
    build_image(&parent);
    build_child_with_backing(&child, &rel, &[]);

    let info = qemu_info_json(&child);
    let backing = info
        .get("backing-filename")
        .unwrap_or_else(|| panic!("child must report a backing filename"));
    assert_eq!(backing, &Value::String(rel));
}

/// Read a big-endian `u64` out of an image on disk.
fn read_be64(path: &Path, off: u64) -> u64 {
    let mut f = std::fs::File::open(path).unwrap();
    let mut b = [0u8; 8];
    f.read_exact_at(&mut b, off).unwrap();
    u64::from_be_bytes(b)
}

/// Where the first L2 table of a real qemu-produced image lives.
///
/// Header byte 40 is `l1_table_offset`; the first L1 entry is a host
/// offset in bits 9..55 with COPIED in bit 63.
fn first_l2_table_offset(path: &Path) -> u64 {
    let l1_off = read_be64(path, 40);
    read_be64(path, l1_off) & HOST_OFFSET_MASK
}

/// Direction 4: the images the reference tool refuses are the ones we
/// refuse.
///
/// Every other test here asks whether we agree with the validator about
/// a *good* image. This asks whether we agree about a bad one, which is
/// the half that decides what happens to a corrupt disk in front of a
/// user. `OFFSET_MASK` covers bits 9..55, so a host offset of 0x200
/// passes through the mask unchanged and names byte 512 — inside the
/// image's own header. The validator says so by name; before this
/// change we opened the image and returned those bytes as the guest's
/// first data cluster with `Ok(())`.
#[test]
fn an_unaligned_l2_entry_is_refused_by_the_validator_and_by_us() {
    let raw = tmp_path("unaligned-src");
    let qcow = tmp_path("unaligned-dst");

    let data = vec![0x5Au8; 4096 * 8];
    std::fs::write(&raw, &data).unwrap();
    qemu_convert_raw_to_qcow2(&raw, &qcow);
    // The image is well-formed until we move one entry.
    qemu_check(&qcow);

    let l2 = first_l2_table_offset(&qcow);
    assert_ne!(l2, 0, "qemu-produced image should have an L2 table");
    patch(&qcow, l2, &(0x200u64 | COPIED).to_be_bytes());

    // The reference tool refuses to convert it.
    let out = run_qemu(&[
        "convert",
        "-f",
        "qcow2",
        "-O",
        "raw",
        qcow.to_str().unwrap(),
        tmp_path("unaligned-out").to_str().unwrap(),
    ]);
    assert!(
        !out.status.success(),
        "the validator was expected to refuse an unaligned L2 entry, but it converted the image"
    );

    // And so do we, rather than serving the header's bytes.
    let r = Qcow2Reader::open(&qcow).unwrap();
    let mut buf = vec![0u8; 512];
    let err = r.read_at(0, &mut buf).unwrap_err();
    assert!(
        matches!(err, qcow2::Error::Corrupt(_)),
        "expected a refusal, got {err:?} with buf starting {:02x?}",
        &buf[..8]
    );
}

/// Take an internal snapshot with the reference tool.
fn qemu_snapshot_create(path: &Path, name: &str) {
    assert_qemu(&["snapshot", "-c", name, path.to_str().unwrap()]);
}

/// Convert one internal snapshot's view of the disk out to raw.
fn qemu_convert_snapshot_to_raw(qcow: &Path, name: &str, raw: &Path) {
    assert_qemu(&[
        "convert",
        "-f",
        "qcow2",
        "-l",
        &format!("snapshot.name={name}"),
        "-O",
        "raw",
        qcow.to_str().unwrap(),
        raw.to_str().unwrap(),
    ]);
}

/// Direction 5: a write into an image that has an internal snapshot
/// must leave the snapshot holding what it was taken to hold.
///
/// This is the case the format's copy-on-write exists for, and it
/// cannot be checked without a real snapshot: when one is taken, the
/// active L1 and the snapshot's L1 point at the *same* L2 table and
/// COPIED is cleared on the L1 entry to say so. Writing into that
/// shared table changes the snapshot's view. A synthetic fixture that
/// only bumps a data cluster's refcount never reaches it.
///
/// The two assertions are independent and both used to fail. The
/// validator reported `ERROR cluster 11 refcount=1 reference=2` and a
/// leaked cluster; the snapshot's own view came back as the bytes we
/// had just written.
#[test]
fn a_write_leaves_an_internal_snapshot_holding_what_it_held() {
    let raw = tmp_path("snap-src");
    let img = tmp_path("snap-img");
    let snap_out = tmp_path("snap-view");

    let mut data = vec![0u8; 65_536 * 4];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    std::fs::write(&raw, &data).unwrap();
    qemu_convert_raw_to_qcow2(&raw, &img);
    qemu_snapshot_create(&img, "s1");
    qemu_check(&img);

    {
        let r = Qcow2Reader::open_rw(&img).unwrap();
        r.write_at(0, &[0xEEu8; 4096]).unwrap();
        r.flush().unwrap();
    }

    // Structurally sound: no refcount mismatch, no leak.
    qemu_check(&img);

    // And the snapshot still holds the pre-write bytes.
    qemu_convert_snapshot_to_raw(&img, "s1", &snap_out);
    let seen = std::fs::read(&snap_out).unwrap();
    assert_eq!(
        &seen[..data.len()],
        &data[..],
        "the snapshot's view changed; first bytes are {:02x?}",
        &seen[..8]
    );

    // The live view is the one that changed.
    let r = Qcow2Reader::open(&img).unwrap();
    let mut live = vec![0u8; 4096];
    r.read_at(0, &mut live).unwrap();
    assert!(live.iter().all(|&b| b == 0xEE), "the write did not land");
}

/// A 3 MiB pattern converted to qcow2 by the reference tool, so the
/// tables are where a real producer puts them.
fn populated_qcow2(name: &str) -> (std::path::PathBuf, Vec<u8>) {
    let raw = tmp_path(&format!("{name}-raw"));
    let qcow = tmp_path(name);
    let data = (0..(3 * 1024 * 1024))
        .map(|i| (i % 251) as u8)
        .collect::<Vec<u8>>();
    std::fs::write(&raw, &data).unwrap();
    qemu_convert_raw_to_qcow2(&raw, &qcow);
    let _ = std::fs::remove_file(&raw);
    (qcow, data)
}

/// A table offset that is not cluster-aligned is refused by the
/// validator and by us.
///
/// Each offset is moved half a cluster past its real value, so the
/// table is still inside the image and only its alignment is wrong —
/// which is what separates this from the "past the end" check that
/// already existed and was catching these by accident of image size.
///
/// Both directions are asserted, so the test cannot pass by the patch
/// failing to take effect.
#[test]
fn an_unaligned_table_offset_is_refused_by_the_validator_and_by_us() {
    for (field, header_offset, qemu_says) in [
        ("l1_table_offset", 40u64, "Active L1 table offset invalid"),
        (
            "refcount_table_offset",
            48u64,
            "Reference count table offset invalid",
        ),
    ] {
        let (qcow, data) = populated_qcow2(field);

        // Positive control: untouched, both of us read it.
        qemu_check(&qcow);
        {
            let r = Qcow2Reader::open(&qcow).unwrap();
            let mut buf = vec![0u8; 4096];
            r.read_at(0, &mut buf).unwrap();
            assert_eq!(buf, data[..4096], "{field}: the untouched image must read");
        }

        let real = read_be64(&qcow, header_offset);
        assert_ne!(real, 0, "{field}: qemu should have written a real offset");
        patch(&qcow, header_offset, &(real + 0x200).to_be_bytes());

        let refusal = run_qemu(&["info", qcow.to_str().unwrap()]);
        let stderr = String::from_utf8_lossy(&refusal.stderr).into_owned();
        assert!(
            !refusal.status.success() && stderr.contains(qemu_says),
            "precondition: the validator must refuse {field}, got {stderr:?}"
        );

        match Qcow2Reader::open(&qcow) {
            Err(qcow2::Error::Corrupt(msg)) => assert!(
                msg.contains("cluster-aligned"),
                "{field}: the refusal must name the alignment, got {msg:?}"
            ),
            Err(other) => panic!("{field}: expected Corrupt, got {other:?}"),
            Ok(r) => {
                let mut buf = vec![0u8; 16];
                let read = r.read_at(0, &mut buf);
                panic!(
                    "{field}: opened an image the validator refuses; read {read:?} gave {:02x?} \
                     where the guest holds {:02x?}",
                    buf,
                    &data[..16]
                );
            }
        }
        // And read-write, which is where it costs more than a bad read.
        assert!(
            Qcow2Reader::open_rw(&qcow).is_err(),
            "{field}: must not open read-write either"
        );
        let _ = std::fs::remove_file(&qcow);
    }
}

const INCOMPAT_FEATURES_OFFSET: u64 = 72;
const INCOMPAT_DIRTY: u64 = 1 << 0;
const INCOMPAT_CORRUPT: u64 = 1 << 1;

/// The two advisory bits are advisory to a reader and not to a writer,
/// and the reference tool draws the line in the same place.
///
/// Measured, on an image that is otherwise sound:
///
/// ```text
/// CORRUPT: qemu-img info      -> reports the image, "corrupt: true"
///          qemu-img snapshot  -> "Image is corrupt; cannot be opened read/write"
/// DIRTY:   qemu-img snapshot  -> succeeds, and the bit reads 0x0 afterwards,
///                                because it repaired the refcounts first
/// ```
///
/// We cannot repair, so refusing is what this crate can honestly do.
/// The read half is asserted too, because refusing to read a flagged
/// image would take away the one thing somebody with a damaged one
/// wants.
#[test]
fn a_flagged_image_still_reads_and_no_longer_takes_writes() {
    for (label, bit) in [("corrupt", INCOMPAT_CORRUPT), ("dirty", INCOMPAT_DIRTY)] {
        let (qcow, data) = populated_qcow2(&format!("flagged-{label}"));

        // Positive control: unflagged, it reads and writes.
        {
            let r = Qcow2Reader::open_rw(&qcow).expect("the unflagged image is writable");
            r.write_at(0, &[0xABu8; 512]).expect("and takes a write");
            r.flush().unwrap();
        }
        qemu_check(&qcow);

        patch(&qcow, INCOMPAT_FEATURES_OFFSET, &bit.to_be_bytes());

        // Reading is still allowed — and still correct past the bytes we
        // just overwrote, so this is not passing on an empty image.
        {
            let r = Qcow2Reader::open(&qcow).expect("a flagged image must still be readable");
            let mut buf = vec![0u8; 4096];
            r.read_at(65536, &mut buf).unwrap();
            assert_eq!(
                buf,
                data[65536..65536 + 4096],
                "{label}: the read must be right"
            );
        }

        // Writing is not.
        match Qcow2Reader::open_rw(&qcow) {
            Err(qcow2::Error::Unsupported(m)) => assert!(
                m.contains(label),
                "{label}: the refusal must name the flag, got {m:?}"
            ),
            Err(other) => panic!("{label}: expected Unsupported, got {other:?}"),
            Ok(_) => panic!("{label}: opened read-write an image the validator refuses"),
        }

        let _ = std::fs::remove_file(&qcow);
    }
}

const BACKING_FILE_OFFSET: u64 = 8;
const BACKING_FILE_SIZE: u64 = 16;

/// The backing chain is keyed off the offset, and the offset is bounded
/// — and we agree with the reference tool about both.
///
/// Three shapes, each asserted against `qemu-img info` as well as
/// against us, so the test cannot pass by a patch failing to take.
#[test]
fn the_backing_chain_is_keyed_and_bounded_like_the_validator_does_it() {
    // 1. A size with no offset is not a backing file.
    //
    //    The validator reports such an image with no backing file. We
    //    read `backing_file_size` bytes from offset 0 — the header's own
    //    magic — and refused the image with BadBackingPath.
    {
        let p = tmp_path("backing-size-no-offset");
        qemu_create(&p, "4M");
        patch(&p, BACKING_FILE_SIZE, &12u32.to_be_bytes());

        let info = run_qemu(&["info", p.to_str().unwrap()]);
        assert!(info.status.success(), "the validator opens this image");
        assert!(
            !String::from_utf8_lossy(&info.stdout).contains("backing file"),
            "and reports no backing file"
        );
        Qcow2Reader::open(&p).expect("so must we");
        let _ = std::fs::remove_file(&p);
    }

    // 1b. An offset with no length is not a backing file either.
    //
    //     The reference tool opens such an image and reports no backing
    //     file. Refusing it — which an offset-only gate does — rejects a
    //     legal image, which is the same mistake as case 1 made while
    //     fixing case 1.
    {
        let p = tmp_path("backing-offset-no-size");
        qemu_create(&p, "4M");
        patch(&p, BACKING_FILE_OFFSET, &0x50u64.to_be_bytes());
        patch(&p, BACKING_FILE_SIZE, &0u32.to_be_bytes());

        let info = run_qemu(&["info", p.to_str().unwrap()]);
        assert!(
            info.status.success(),
            "the validator opens an image whose backing length is zero"
        );
        assert!(
            !String::from_utf8_lossy(&info.stdout).contains("backing file"),
            "and reports no backing file"
        );
        Qcow2Reader::open(&p).expect("so must we");
        let _ = std::fs::remove_file(&p);
    }

    // 2. An offset past the first cluster is refused.
    {
        let p = tmp_path("backing-offset-far");
        qemu_create(&p, "4M");
        patch(&p, BACKING_FILE_OFFSET, &0x50000u64.to_be_bytes());
        patch(&p, BACKING_FILE_SIZE, &8u32.to_be_bytes());

        let refusal = run_qemu(&["info", p.to_str().unwrap()]);
        assert!(
            !refusal.status.success()
                && String::from_utf8_lossy(&refusal.stderr).contains("backing file offset"),
            "precondition: the validator must refuse it"
        );
        match Qcow2Reader::open(&p) {
            Err(qcow2::Error::Corrupt(m)) => {
                assert!(m.contains("first cluster"), "got {m:?}")
            }
            Err(other) => panic!("expected Corrupt, got {other:?}"),
            Ok(_) => panic!("opened an image the validator refuses"),
        }
        let _ = std::fs::remove_file(&p);
    }

    // 3. The one that costs something: a real parent named from bytes
    //    inside a data cluster, which is to say bytes the guest wrote.
    //    Before this we opened the image AND opened the parent, so a
    //    guest that can write its own disk chose which host file this
    //    reader opened.
    {
        let parent = tmp_path("backing-guest-parent");
        let child = tmp_path("backing-guest-child");
        qemu_create(&parent, "4M");
        qemu_create(&child, "4M");
        let name = parent.file_name().unwrap().to_string_lossy().into_owned();
        patch(&child, 0x50000, name.as_bytes());
        patch(&child, BACKING_FILE_OFFSET, &0x50000u64.to_be_bytes());
        patch(
            &child,
            BACKING_FILE_SIZE,
            &(name.len() as u32).to_be_bytes(),
        );

        let refusal = run_qemu(&["info", child.to_str().unwrap()]);
        assert!(
            !refusal.status.success(),
            "precondition: the validator must refuse a parent named from a data cluster"
        );
        assert!(
            Qcow2Reader::open(&child).is_err(),
            "we must not open a parent the guest chose"
        );
        let _ = std::fs::remove_file(&child);
        let _ = std::fs::remove_file(&parent);
    }
}

/// The positive control: a real backing chain still opens and still
/// reads through to its parent.
///
/// Without this, refusing every backing file would pass every assertion
/// above.
#[test]
fn a_real_backing_chain_still_opens_and_reads_through() {
    let raw = tmp_path("chain-src");
    let parent = tmp_path("chain-parent");
    let child = tmp_path("chain-child");

    let data: Vec<u8> = (0..(4096 * 4)).map(|i| (i % 251) as u8).collect();
    std::fs::write(&raw, &data).unwrap();
    qemu_convert_raw_to_qcow2(&raw, &parent);
    assert_qemu(&[
        "create",
        "-f",
        "qcow2",
        "-b",
        parent.to_str().unwrap(),
        "-F",
        "qcow2",
        child.to_str().unwrap(),
    ]);

    let r = Qcow2Reader::open(&child).expect("a real chain must open");
    let mut buf = vec![0u8; data.len()];
    r.read_at(0, &mut buf).unwrap();
    assert_eq!(buf, data, "the child must read through to its parent");

    let _ = std::fs::remove_file(&child);
    let _ = std::fs::remove_file(&parent);
    let _ = std::fs::remove_file(&raw);
}
