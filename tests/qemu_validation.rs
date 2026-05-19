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

use qcow2::Qcow2Reader;
use std::path::{Path, PathBuf};
use std::process::Command;

const QEMU_IMG: &str = "qemu-img";

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "qcow2-validate-{name}-{}-{:p}.img",
        std::process::id(),
        &name as *const _
    ));
    p
}

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
