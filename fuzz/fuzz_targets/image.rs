#![no_main]
//! A whole image, opened and read.
//!
//! Every field a qcow2 image controls is an offset or a shift used in
//! arithmetic: `cluster_bits`, `l1_size`, `l1_table_offset`,
//! `refcount_table_offset`. A cluster size that shifts out to zero
//! divides by zero on the first read; an L2 entry offset that is not
//! cluster-aligned indexes somewhere it should not. The 2026-09-06
//! hardening wave found that class of defect across this family by
//! hand.
use libfuzzer_sys::fuzz_target;
use qcow2_fuzz::walk;

fuzz_target!(|data: &[u8]| {
    walk(data);
});
