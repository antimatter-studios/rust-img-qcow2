//! `rust-img-qcow2`: the command-line tool for QCOW2 images, one multi-call
//! binary.
//!
//! Installed as `rust-img-qcow2` and linked as `img.qcow2`; see `common` for
//! the dispatch and the output contract every tool shares, and `qcow2` for
//! the tool itself.

// The shared plumbing is a library in waiting (see its module docs): its
// API is whole, and a piece this repository does not call yet is not dead,
// it is the part another format's tools will.
#[allow(dead_code)]
mod common;
mod qcow2;

use std::process::ExitCode;

static FAMILY: common::Family = common::Family {
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
    common::main(&FAMILY)
}
