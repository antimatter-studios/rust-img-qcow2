//! `img.qcow2 write` refuses input that cannot be written before writing
//! any of it, and leaves the image as it was.
//!
//! The tool reads its input from stdin: a regular file redirected there is
//! measured before a byte is read, and a pipe is read no further than one
//! byte past what fits. What the written bytes look like to another
//! implementation is tests/cli/test-write.sh's question, against qemu-img.

mod common;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const TOOL: &str = env!("CARGO_BIN_EXE_rust-img-qcow2");

fn img(args: &[&str]) -> Command {
    let mut cmd = Command::new(TOOL);
    cmd.arg("img").args(args);
    cmd
}

fn write_from_file(image: &Path, offset: &str, input: &Path) -> Output {
    img(&[image.to_str().unwrap(), "write", "--offset", offset])
        .stdin(std::fs::File::open(input).unwrap())
        .output()
        .unwrap()
}

fn virtual_size(image: &Path) -> u64 {
    img_qcow2::Qcow2Reader::open(image).unwrap().virtual_size()
}

#[test]
fn write_refuses_an_input_past_the_virtual_disk_by_its_length() {
    let image = common::tmp_path("cli-past-end");
    common::build_image(&image);
    let before = std::fs::read(&image).unwrap();
    let size = virtual_size(&image);

    let input = common::tmp_path("cli-past-end-input");
    std::fs::File::create(&input)
        .unwrap()
        .set_len(size + 1)
        .unwrap();
    let wrote = write_from_file(&image, "0", &input);
    let stderr = String::from_utf8_lossy(&wrote.stderr);
    assert_eq!(
        wrote.status.code(),
        Some(1),
        "an oversized input was accepted: {stderr}"
    );
    // Off Unix stdin is always read as a pipe (see `stdin_file`), so the
    // refusal there is the pipe's: the bytes on stdin outnumber the room.
    assert!(
        stderr.contains("run past") || (cfg!(not(unix)) && stderr.contains("more than")),
        "refused, but not by its length: {stderr}"
    );
    assert!(
        std::fs::read(&image).unwrap() == before,
        "the refused write changed the image"
    );

    // And an input that fits, into an allocated cluster, still writes.
    std::fs::write(&input, b"fits").unwrap();
    let wrote = write_from_file(&image, "512", &input);
    assert!(
        wrote.status.success(),
        "{}",
        String::from_utf8_lossy(&wrote.stderr)
    );
    let r = img_qcow2::Qcow2Reader::open(&image).unwrap();
    let mut back = [0u8; 4];
    r.read_at(512, &mut back).unwrap();
    assert_eq!(&back, b"fits");
    drop(r);
    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_file(&input);
}

/// Input through a pipe has no length to check first; it is read no
/// further than one byte past what fits, and refused before any is written.
#[test]
fn write_refuses_a_piped_input_past_the_virtual_disk_before_writing() {
    let image = common::tmp_path("cli-pipe-past-end");
    common::build_image(&image);
    let before = std::fs::read(&image).unwrap();
    let size = virtual_size(&image) as usize;

    let mut child = img(&[image.to_str().unwrap(), "write", "--offset", "512"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // More than fits; the tool may stop reading before all of it is sent.
    let _ = stdin.write_all(&vec![0xAB; size * 2]);
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "an oversized pipe was accepted: {stderr}"
    );
    assert!(stderr.contains("more than"), "{stderr}");
    assert!(
        std::fs::read(&image).unwrap() == before,
        "the refused write changed the image"
    );
    let _ = std::fs::remove_file(&image);
}

/// An image given as its own input is refused, and left as it was.
#[cfg(unix)]
#[test]
fn write_refuses_the_image_as_its_own_input() {
    let image = common::tmp_path("cli-self");
    common::build_image(&image);
    let before = std::fs::read(&image).unwrap();
    let wrote = write_from_file(&image, "0", &image);
    let stderr = String::from_utf8_lossy(&wrote.stderr);
    assert!(!wrote.status.success(), "an image was written into itself");
    assert!(stderr.contains("is the image being written"), "{stderr}");
    assert!(
        std::fs::read(&image).unwrap() == before,
        "the refused write changed the image"
    );
    let _ = std::fs::remove_file(&image);
}
