//! Pure-Rust QCOW2 reader.
//!
//! Supported now:
//! - Uncompressed and zlib-compressed clusters
//! - Backing-file chain (recursive)
//! - Sparse / v3 zero-flagged clusters
//!
//! Implements [`fs_core::BlockRead`] so any consumer that takes a
//! `BlockRead` (partition probe, fs driver, slice adapter) can drive a
//! `Qcow2Reader` directly. The C ABI in [`capi`] returns a
//! [`fs_core::ffi::FsCoreDevice`] handle for the same reason.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod capi;
pub mod error;
pub mod header;
pub mod reader;

pub use error::{Error, Result};
pub use header::Header;
pub use reader::Qcow2Reader;
