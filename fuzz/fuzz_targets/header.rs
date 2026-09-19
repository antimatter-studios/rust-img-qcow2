#![no_main]
//! The header alone, including the extension area behind it.
//!
//! The backing-file name is an offset plus a length, and the header
//! extensions are a chain of length-prefixed records -- both read
//! before anything about the image has been established.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = qcow2::Header::parse(data) {
        let _ = header.check_supported();
        let _ = header.check_writable();
        let _ = header.l2_entries();
        let _ = header.is_corrupt();
        let _ = header.has_dirty_refcounts();
    }
});
