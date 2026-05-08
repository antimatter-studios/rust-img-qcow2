//! C ABI for the qcow2 reader.
//!
//! Single entry point: [`qcow2_open`] returns a generic
//! [`FsCoreDevice`][fs_core::ffi::FsCoreDevice] handle so the rest of the
//! work (partition probe, FS sniff, mount) goes through the same handle
//! type every sister crate expects. There is intentionally no
//! qcow2-specific handle type at the C level — once the file is opened
//! it's just a block device.

#![allow(clippy::missing_safety_doc)]

use crate::Qcow2Reader;
use fs_core::ffi::{set_last_error, FsCoreDevice};
use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::AssertUnwindSafe;
use std::ptr;
use std::sync::Arc;

/// Open `path` (NUL-terminated UTF-8) as a QCOW2 image and return a
/// generic device handle. On failure returns NULL; consult
/// `fs_core_last_error_message()` for detail.
///
/// The caller owns the handle and frees it via `fs_core_device_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qcow2_open(path: *const c_char) -> *mut FsCoreDevice {
    open_path(path, false)
}

/// Open `path` read-write. Subject to the Phase A constraints documented
/// on [`Qcow2Reader::write_at`] — writes succeed only against
/// already-allocated, single-reference, uncompressed clusters; everything
/// else returns `FS_CORE_CUSTOM` with detail in
/// `fs_core_last_error_message()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qcow2_open_rw(path: *const c_char) -> *mut FsCoreDevice {
    open_path(path, true)
}

fn open_path(path: *const c_char, writable: bool) -> *mut FsCoreDevice {
    if path.is_null() {
        set_last_error("path is null");
        return ptr::null_mut();
    }
    let res = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let cstr = unsafe { CStr::from_ptr(path) };
        let s = match cstr.to_str() {
            Ok(s) => s,
            Err(_) => {
                set_last_error("path is not valid UTF-8");
                return ptr::null_mut();
            }
        };
        let reader = if writable {
            Qcow2Reader::open_rw(s)
        } else {
            Qcow2Reader::open(s)
        };
        match reader {
            Ok(r) => FsCoreDevice::into_handle(Arc::new(r)),
            Err(e) => {
                set_last_error(e.to_string());
                ptr::null_mut()
            }
        }
    }));
    match res {
        Ok(p) => p,
        Err(_) => {
            set_last_error("panic in qcow2_open");
            ptr::null_mut()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_core::ffi::{
        fs_core_device_close, fs_core_device_read_at, fs_core_device_size_bytes,
        fs_core_last_error_message, FsCoreErrorCode,
    };
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;

    const QCOW2_MAGIC: u32 = 0x5146_49fb;

    /// Build the minimal 16 KB qcow2 v3 image used by the existing
    /// integration tests, just inline here so the FFI smoke is
    /// self-contained.
    fn build_image(path: &PathBuf) {
        const COPIED: u64 = 1u64 << 63;
        const CLUSTER: u64 = 4096;
        const VIRT: u64 = CLUSTER * 4;
        const L1_OFF: u64 = CLUSTER;
        const L2_OFF: u64 = CLUSTER * 2;
        const D0_OFF: u64 = CLUSTER * 3;

        let mut f = File::create(path).unwrap();
        f.set_len(CLUSTER * 4).unwrap();

        let mut hdr = [0u8; 4096];
        hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
        hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
        hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
        hdr[24..32].copy_from_slice(&VIRT.to_be_bytes());
        hdr[36..40].copy_from_slice(&1u32.to_be_bytes());
        hdr[40..48].copy_from_slice(&L1_OFF.to_be_bytes());
        hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
        hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(&hdr).unwrap();

        let mut l1 = [0u8; 4096];
        l1[0..8].copy_from_slice(
            &((L2_OFF & 0x00ff_ffff_ffff_fe00) | COPIED).to_be_bytes(),
        );
        f.seek(SeekFrom::Start(L1_OFF)).unwrap();
        f.write_all(&l1).unwrap();

        let mut l2 = [0u8; 4096];
        l2[0..8].copy_from_slice(
            &((D0_OFF & 0x00ff_ffff_ffff_fe00) | COPIED).to_be_bytes(),
        );
        f.seek(SeekFrom::Start(L2_OFF)).unwrap();
        f.write_all(&l2).unwrap();

        let d0 = [0xAAu8; 4096];
        f.seek(SeekFrom::Start(D0_OFF)).unwrap();
        f.write_all(&d0).unwrap();
    }

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "qcow2_capi_{}_{}_{name}.qcow2",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn open_then_read_via_fs_core_handle() {
        let path = tmp_path("smoke");
        build_image(&path);
        let cpath = CString::new(path.to_string_lossy().to_string()).unwrap();

        let h = unsafe { qcow2_open(cpath.as_ptr()) };
        assert!(!h.is_null(), "qcow2_open returned NULL");

        unsafe {
            assert_eq!(fs_core_device_size_bytes(h), 16384);

            let mut buf = [0u8; 4096];
            let rc = fs_core_device_read_at(h, 0, buf.as_mut_ptr(), buf.len());
            assert_eq!(rc, FsCoreErrorCode::Ok);
            assert!(buf.iter().all(|&b| b == 0xAA));

            // Read past the virtual disk -> OutOfBounds.
            let rc = fs_core_device_read_at(h, 16384, buf.as_mut_ptr(), 16);
            assert_eq!(rc, FsCoreErrorCode::OutOfBounds);

            fs_core_device_close(h);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nonexistent_path_returns_null_with_message() {
        let cpath = CString::new("/no/such/file/we/hope.qcow2").unwrap();
        let h = unsafe { qcow2_open(cpath.as_ptr()) };
        assert!(h.is_null());
        let msg = fs_core_last_error_message();
        assert!(!msg.is_null());
    }

    #[test]
    fn rw_open_writes_to_allocated_cluster_via_fs_core_handle() {
        use fs_core::ffi::{fs_core_device_flush, fs_core_device_is_writable, fs_core_device_write_at};

        let path = tmp_path("rw_smoke");
        build_image(&path);
        let cpath = CString::new(path.to_string_lossy().to_string()).unwrap();

        let h = unsafe { qcow2_open_rw(cpath.as_ptr()) };
        assert!(!h.is_null(), "qcow2_open_rw returned NULL");

        unsafe {
            assert!(fs_core_device_is_writable(h));

            // Overwrite four bytes inside virt cluster 0 (originally 0xAA).
            let payload = [0xDEu8, 0xAD, 0xBE, 0xEF];
            let rc = fs_core_device_write_at(h, 200, payload.as_ptr(), payload.len());
            assert_eq!(rc, FsCoreErrorCode::Ok);
            assert_eq!(fs_core_device_flush(h), FsCoreErrorCode::Ok);

            // Read it back via the same handle.
            let mut readback = [0u8; 4];
            let rc = fs_core_device_read_at(h, 200, readback.as_mut_ptr(), readback.len());
            assert_eq!(rc, FsCoreErrorCode::Ok);
            assert_eq!(readback, payload);

            // Writing into the unallocated cluster errors as Custom because
            // this fixture has no refcount table — the allocator has nowhere
            // to claim a fresh cluster from. (The full-fixture allocator
            // path is exercised by the Phase B unit tests.)
            let rc =
                fs_core_device_write_at(h, 4096, payload.as_ptr(), payload.len());
            assert_eq!(rc, FsCoreErrorCode::Custom);

            fs_core_device_close(h);
        }
        let _ = std::fs::remove_file(&path);
    }
}
