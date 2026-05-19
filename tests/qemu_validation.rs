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
