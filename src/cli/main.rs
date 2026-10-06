//! `rust-img-qcow2`: the command-line tool for QCOW2 images, one multi-call
//! binary.
//!
//! Installed as `rust-img-qcow2` and linked as `img.qcow2`. The dispatch and the
//! output contract every tool shares are `fs_core::cli` (rust-fs-core's `cli`
//! feature); `qcow2` is the tool itself.

mod qcow2;

use fs_core::cli;
use std::process::ExitCode;

static FAMILY: cli::Family = cli::Family {
    repo: "rust-img-qcow2",
    crate_name: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    about: "QCOW2 tools: report, read and write a QCOW2 disk image without a hypervisor",
    install_hints: &[
        "`chore cli:install` from a checkout of this repository",
        "`brew install antimatter-studios/tap/rust-img-qcow2`",
    ],
    tools: &[qcow2::img::TOOL],
};

fn main() -> ExitCode {
    cli::main(&FAMILY)
}
