//! Pure-Rust QCOW2 reader and writer.
//!
//! Supported now:
//! - Uncompressed clusters, read and written
//! - zlib- and zstd-compressed clusters, read
//! - Backing-file chain (recursive), including copy-up on write
//! - Sparse / v3 zero-flagged clusters
//! - Cluster allocation and refcount maintenance (`open_rw`)
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
pub use reader::{ClusterStatus, Extent, ExtentIter, Qcow2Reader};
