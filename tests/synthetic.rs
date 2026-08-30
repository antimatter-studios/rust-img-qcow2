//! End-to-end tests on hand-crafted qcow2 v3 images. The synthetic-image
//! builders (`build_image`, `build_compressed_image`,
//! `build_child_with_backing`, `build_zstd_compressed_image`) live in
//! `tests/common/mod.rs` and are exercised in two places:
//!
//! 1. **Here** — open with our reader, assert observed bytes match what
//!    the builder put in.
//! 2. **`tests/qemu_validation.rs`** — same builders, but the output is
//!    passed to `qemu-img check` / `qemu-img info` so we cross-check
//!    that our hand-built bytes are actually spec-valid, not just
//!    self-consistent with our own reader.
//!
//! Standard layout (cluster_size = 4096):
//!
//!   cluster 0   header
//!   cluster 1   L1 table (1 entry)
//!   cluster 2   L2 table
//!   cluster 3   data backing virtual cluster 0   (filled 0xAA)
//!   cluster 4   data backing virtual cluster 2   (filled 0xBB)
//!
//!   virt 0  -> data at cluster 3                 (allocated)
//!   virt 1  -> unallocated                       (reads zeros)
//!   virt 2  -> data at cluster 4                 (allocated)
//!   virt 3  -> v3 zero flag                      (reads zeros)

mod common;

use common::*;
use qcow2::Qcow2Reader;
use std::fs::File;
use std::path::PathBuf;

#[test]
fn header_round_trip() {
    let path = tmp_path("hdr");
    build_image(&path);

    let r = Qcow2Reader::open(&path).unwrap();
    assert_eq!(r.version(), 3);
    assert_eq!(r.cluster_size(), 4096);
    assert_eq!(r.virtual_size(), VIRT_SIZE);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn allocated_cluster_reads_back_data() {
    let path = tmp_path("alloc");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    let mut buf = vec![0u8; 4096];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xAA),
        "virt cluster 0 should be all 0xAA"
    );

    let mut buf = vec![0u8; 4096];
    r.read_at(8192, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xBB),
        "virt cluster 2 should be all 0xBB"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unallocated_cluster_reads_zeros() {
    let path = tmp_path("sparse");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    let mut buf = vec![0xFFu8; 4096];
    r.read_at(4096, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0),
        "unallocated cluster must read as zeros"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn v3_zero_flag_reads_zeros() {
    let path = tmp_path("zero");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    let mut buf = vec![0xFFu8; 4096];
    r.read_at(12288, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0),
        "v3 zero-flagged cluster must read as zeros"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn cross_cluster_read() {
    let path = tmp_path("cross");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    // Read straddling virt cluster 0 (0xAA) and virt cluster 1 (sparse zeros).
    let mut buf = vec![0u8; 8];
    r.read_at(4092, &mut buf).unwrap();
    assert_eq!(&buf[0..4], &[0xAA; 4]);
    assert_eq!(&buf[4..8], &[0x00; 4]);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn read_past_end_errors() {
    let path = tmp_path("eof");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = vec![0u8; 16];
    let err = r.read_at(VIRT_SIZE - 8, &mut buf).unwrap_err();
    matches!(err, qcow2::Error::OutOfBounds { .. });
    let _ = std::fs::remove_file(&path);
}

#[test]
fn compressed_cluster_round_trip() {
    let path = tmp_path("compressed");
    build_compressed_image(&path, 0xCC);

    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = vec![0u8; 4096];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xCC),
        "decompressed cluster must match input pattern"
    );

    // Cache hit: read again, smaller window.
    let mut buf2 = vec![0u8; 16];
    r.read_at(2048, &mut buf2).unwrap();
    assert!(buf2.iter().all(|&b| b == 0xCC));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn backing_fallthrough_returns_parent_data() {
    let (parent, child, rel) = pair_paths("fallthrough");
    // Parent has data 0xAA at virt cluster 0.
    build_image(&parent);
    // Child's L2 entry 0 = unallocated → defer to parent.
    build_child_with_backing(&child, &rel, &[]);

    let r = Qcow2Reader::open(&child).unwrap();
    assert!(r.has_backing());

    let mut buf = vec![0u8; 4096];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xAA),
        "unallocated child cluster should fall through to parent's 0xAA"
    );

    let _ = std::fs::remove_file(&parent);
    let _ = std::fs::remove_file(&child);
}

#[test]
fn child_zero_flag_suppresses_backing() {
    let (parent, child, rel) = pair_paths("zero_blocks");
    build_image(&parent);
    // Child L2[0] explicitly zero — must NOT defer to parent.
    let entry = COPIED | L2_FLAG_ZERO;
    build_child_with_backing(&child, &rel, &[(0, entry)]);

    let r = Qcow2Reader::open(&child).unwrap();
    let mut buf = vec![0xFFu8; 4096];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0),
        "v3 zero flag must override the backing chain"
    );

    let _ = std::fs::remove_file(&parent);
    let _ = std::fs::remove_file(&child);
}

#[test]
fn backing_too_deep_for_self_reference() {
    // Build a qcow2 whose backing path points at itself, then confirm we
    // reject before exhausting the stack.
    let dir = std::env::temp_dir();
    let path = dir.join(format!("qcow2_cycle_{}.qcow2", std::process::id()));
    let rel = path.file_name().unwrap().to_string_lossy().into_owned();
    build_child_with_backing(&path, &rel, &[]);

    match Qcow2Reader::open(&path) {
        Err(qcow2::Error::BackingTooDeep) => {}
        Err(e) => panic!("expected BackingTooDeep, got {e:?}"),
        Ok(_) => panic!("expected BackingTooDeep, opened successfully"),
    }

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Phase A: write support against already-allocated, uncompressed,
// single-reference clusters.
// ---------------------------------------------------------------------------

#[test]
fn write_to_allocated_cluster_round_trip() {
    let path = tmp_path("write_alloc");
    build_image(&path);

    // Open RW, overwrite a slice of virt cluster 0 (was 0xAA), flush, close.
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        assert!(r.is_writable());
        r.write_at(64, &[0x11, 0x22, 0x33, 0x44]).unwrap();
        r.flush().unwrap();
    }

    // Re-open read-only and verify the bytes landed.
    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 8];
    r.read_at(60, &mut buf).unwrap();
    assert_eq!(buf, [0xAA, 0xAA, 0xAA, 0xAA, 0x11, 0x22, 0x33, 0x44]);

    // Bytes outside the written range stay 0xAA.
    let mut tail = [0u8; 4];
    r.read_at(72, &mut tail).unwrap();
    assert_eq!(tail, [0xAA; 4]);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_to_unallocated_cluster_allocates_phase_b() {
    let path = tmp_path("write_alloc_b_sparse");
    build_image(&path);

    // virt cluster 1 is unallocated. Writing should allocate a fresh
    // host cluster, place the payload at offset 200 inside it, and
    // surround it with zeros.
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(4096 + 200, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        r.flush().unwrap();
    }

    // Re-open RO and verify.
    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 4];
    r.read_at(4096 + 200, &mut buf).unwrap();
    assert_eq!(buf, [0xDE, 0xAD, 0xBE, 0xEF]);

    // Bytes outside the written slice in this cluster are zero (the
    // newly-allocated cluster was zero-initialised).
    let mut head = [0xFFu8; 8];
    r.read_at(4096, &mut head).unwrap();
    assert!(head.iter().all(|&b| b == 0));
    let mut tail = [0xFFu8; 8];
    r.read_at(4096 + 204, &mut tail).unwrap();
    assert!(tail.iter().all(|&b| b == 0));

    let _ = std::fs::remove_file(&path);
}

/// Allocating on write must copy the backing chain up first.
///
/// While the child's L2 entry is zero, virt cluster 0 reads *through* to
/// the parent. The instant the write repoints that entry at a real host
/// cluster the fall-through stops, so every byte the caller did not write
/// has to already be sitting in the new cluster. Zero-filling it instead
/// silently throws the parent's data away.
#[test]
fn write_to_unallocated_cluster_of_backed_image_copies_up_from_parent() {
    let (parent, child, rel) = pair_paths("backed_copy_up");
    // Parent's virt cluster 0 is a full cluster of 0xAA.
    build_image(&parent);
    // Child has no L2 entries at all, so virt cluster 0 is unallocated.
    build_child_with_backing(&child, &rel, &[]);

    {
        let r = Qcow2Reader::open_rw(&child).unwrap();
        assert!(r.has_backing());
        r.write_at(200, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&child).unwrap();
    assert_eq!(
        r.cluster_status_at(0).unwrap(),
        qcow2::ClusterStatus::Allocated,
        "the write must have allocated the cluster in the child"
    );

    let mut cluster = vec![0u8; CLUSTER_SIZE as usize];
    r.read_at(0, &mut cluster).unwrap();

    assert_eq!(
        &cluster[200..204],
        &[0xDE, 0xAD, 0xBE, 0xEF],
        "the caller's payload"
    );
    assert!(
        cluster[..200].iter().all(|&b| b == 0xAA),
        "head of the cluster must still hold the parent's 0xAA, got {:02x?}",
        &cluster[..8]
    );
    assert!(
        cluster[204..].iter().all(|&b| b == 0xAA),
        "tail of the cluster must still hold the parent's 0xAA, got {:02x?}",
        &cluster[204..212]
    );

    let _ = std::fs::remove_file(&parent);
    let _ = std::fs::remove_file(&child);
}

/// The flip side: a v3 zero-flagged cluster genuinely reads as zeros and
/// must *not* pick the parent's data up on write, even though the parent
/// has data at that offset. Zero and Unallocated are not the same case.
#[test]
fn write_to_zero_flagged_cluster_of_backed_image_does_not_copy_up() {
    let (parent, child, rel) = pair_paths("backed_zero_no_copy_up");
    // Parent's virt cluster 0 is a full cluster of 0xAA.
    build_image(&parent);
    // Child explicitly zeroes virt cluster 0, overriding the chain.
    build_child_with_backing(&child, &rel, &[(0, COPIED | L2_FLAG_ZERO)]);

    {
        let r = Qcow2Reader::open_rw(&child).unwrap();
        r.write_at(200, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&child).unwrap();
    let mut cluster = vec![0xFFu8; CLUSTER_SIZE as usize];
    r.read_at(0, &mut cluster).unwrap();

    assert_eq!(&cluster[200..204], &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert!(
        cluster[..200].iter().all(|&b| b == 0) && cluster[204..].iter().all(|&b| b == 0),
        "a zero-flagged cluster must stay zeros around the payload, not adopt the parent's 0xAA"
    );

    let _ = std::fs::remove_file(&parent);
    let _ = std::fs::remove_file(&child);
}

#[test]
fn write_to_zero_flagged_cluster_allocates_phase_b() {
    let path = tmp_path("write_alloc_b_zero");
    build_image(&path);

    // virt cluster 3 has the v3 zero flag set; same allocation path
    // applies.
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(12288, &[0x55; 32]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 32];
    r.read_at(12288, &mut buf).unwrap();
    assert!(buf.iter().all(|&b| b == 0x55));

    // Tail of this cluster reads as zeros (initialised, not still-flagged).
    let mut tail = [0xFFu8; 16];
    r.read_at(12288 + 32, &mut tail).unwrap();
    assert!(tail.iter().all(|&b| b == 0));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn cross_cluster_write_spanning_alloc_and_unalloc() {
    let path = tmp_path("write_cross_alloc");
    build_image(&path);

    // virt 0 (allocated, 0xAA) into virt 1 (unallocated). The write
    // straddles the boundary at offset 4096; both clusters need
    // attention — the first is overwritten in place, the second is
    // freshly allocated.
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(4090, &[0x77u8; 12]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 16];
    r.read_at(4088, &mut buf).unwrap();
    // Bytes 0..2 untouched (still 0xAA), 2..14 are the 0x77 payload,
    // 14..16 are the now-allocated virt 1's zero tail.
    assert_eq!(&buf[0..2], &[0xAA, 0xAA]);
    assert_eq!(&buf[2..14], &[0x77; 12]);
    assert_eq!(&buf[14..16], &[0x00, 0x00]);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_into_unallocated_l1_entry_allocates_l2_table() {
    // Build an image whose virtual size requires *two* L1 entries (each
    // L1 entry covers cluster_size * (cluster_size/8) = 4096 * 512 = 2 MiB
    // of virtual space at our fixture's 4K clusters). We size the virt
    // disk to 4 MiB and stash an empty second L1 slot — the writer must
    // allocate an L2 table on the fly.
    let path = tmp_path("write_alloc_l2");

    // Custom fixture: same shape as build_image but with l1_size=2 and a
    // 4 MiB virtual disk. Only the first L1 entry is populated; the
    // second is zero, so any write to virt offset >= 2 MiB triggers L2
    // allocation.
    let virt_size = 4u64 * 1024 * 1024;
    let total_clusters = 32u64;
    let total = CLUSTER_SIZE * total_clusters;
    let l1_off = CLUSTER_SIZE;
    let l2_off = CLUSTER_SIZE * 2;
    let data0_off = CLUSTER_SIZE * 3;
    let rt_off = CLUSTER_SIZE * 5;
    let rb_off = CLUSTER_SIZE * 6;

    let mut f = File::create(&path).unwrap();
    f.set_len(total).unwrap();

    // Header: l1_size = 2, refcount table set up.
    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
    hdr[24..32].copy_from_slice(&virt_size.to_be_bytes());
    hdr[36..40].copy_from_slice(&2u32.to_be_bytes()); // l1_size = 2
    hdr[40..48].copy_from_slice(&l1_off.to_be_bytes());
    hdr[48..56].copy_from_slice(&rt_off.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes());
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
    f.write_all_at(&hdr, 0).unwrap();

    // L1 table: entry 0 -> L2 at l2_off (COPIED), entry 1 = 0.
    let mut l1 = [0u8; 4096];
    let l1_entry = (l2_off & 0x00ff_ffff_ffff_fe00) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, l1_off).unwrap();

    // L2 (for L1[0]): entry 0 -> data cluster (COPIED).
    let mut l2 = [0u8; 4096];
    let e0 = (data0_off & 0x00ff_ffff_ffff_fe00) | COPIED;
    l2[0..8].copy_from_slice(&e0.to_be_bytes());
    f.write_all_at(&l2, l2_off).unwrap();

    let mut d0 = [0u8; 4096];
    d0.fill(0xAA);
    f.write_all_at(&d0, data0_off).unwrap();

    // Refcount infrastructure (clusters 0..=6 used, rest free).
    let mut rt = [0u8; 4096];
    rt[0..8].copy_from_slice(&rb_off.to_be_bytes());
    f.write_all_at(&rt, rt_off).unwrap();

    let mut rb = [0u8; 4096];
    for cluster_idx in 0..7u16 {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, rb_off).unwrap();
    drop(f);

    // Write at virt offset 2 MiB — falls in L1[1], which is unallocated.
    // The writer must allocate an L2 table, then a data cluster, then
    // splice in the payload.
    let virt_target = 2u64 * 1024 * 1024;
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(virt_target, &[0xCDu8; 32]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 32];
    r.read_at(virt_target, &mut buf).unwrap();
    assert_eq!(buf, [0xCD; 32]);

    // Surrounding bytes (still inside the new cluster) read as zero.
    let mut head = [0xFFu8; 8];
    r.read_at(virt_target - 8, &mut head).unwrap();
    // Wait — virt_target - 8 is in L1[0]'s territory and that L2 has only
    // entry 0 allocated. virt_target - 8 = 2MiB - 8, which falls in
    // virt cluster (2MiB - 8) / 4096. That's the LAST cluster of L1[0]'s
    // range. L2[511] is unallocated — read should return zeros, and not
    // touch the new cluster.
    assert!(head.iter().all(|&b| b == 0));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_grows_refcount_block() {
    // Phase 5a: when every populated refcount block is full, the writer
    // must allocate a fresh refcount block, point the next free slot in
    // the refcount table at it, then claim a host cluster from it. The
    // fixture has a 1-cluster refcount table (512 entries, each
    // governing one block of 2048 host clusters), but only the first
    // entry is populated. Filling that block forces the writer down the
    // grow path.
    let path = tmp_path("write_grow_refcount");
    build_image(&path);

    // Grow the file enough that the new refcount block + a fresh data
    // cluster have somewhere to land. The grow path places the new
    // block at host cluster idx (block_slot * entries_per_block) =
    // 1 * 2048 = 2048, so the file must be at least 2050 clusters long.
    {
        use std::fs::OpenOptions;
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        f.set_len(CLUSTER_SIZE * 2100).unwrap();
    }

    // Mark every refcount entry in the existing block as used.
    {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut full = vec![0u8; CLUSTER_SIZE as usize];
        for i in 0..(CLUSTER_SIZE as usize / 2) {
            full[i * 2..i * 2 + 2].copy_from_slice(&1u16.to_be_bytes());
        }
        f.seek(SeekFrom::Start(REFCOUNT_BLOCK_OFFSET)).unwrap();
        f.write_all(&full).unwrap();
    }

    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        // virt cluster 1 is unallocated — write triggers allocation,
        // which now succeeds via refcount-block growth.
        r.write_at(4096 + 100, &[0xAB, 0xCD, 0xEF]).unwrap();
        r.flush().unwrap();
    }

    // Re-open RO and confirm the bytes round-trip.
    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 3];
    r.read_at(4096 + 100, &mut buf).unwrap();
    assert_eq!(buf, [0xAB, 0xCD, 0xEF]);

    // Confirm the new refcount block self-references with refcount=1.
    // Per the grow logic the new block lives at host cluster 2048 (slot
    // 1 of the refcount table * 2048 entries per block).
    {
        use std::fs::OpenOptions;
        use std::io::{Read, Seek, SeekFrom};
        let mut f = OpenOptions::new().read(true).open(&path).unwrap();
        // Refcount-table slot 1 should now point at cluster 2048.
        f.seek(SeekFrom::Start(REFCOUNT_TABLE_OFFSET + 8)).unwrap();
        let mut entry = [0u8; 8];
        f.read_exact(&mut entry).unwrap();
        let block_off = u64::from_be_bytes(entry);
        assert_eq!(block_off, CLUSTER_SIZE * 2048);

        // First two entries of the new block should both read as 1
        // (self-reference + the caller's data cluster).
        f.seek(SeekFrom::Start(block_off)).unwrap();
        let mut head = [0u8; 4];
        f.read_exact(&mut head).unwrap();
        assert_eq!(u16::from_be_bytes([head[0], head[1]]), 1);
        assert_eq!(u16::from_be_bytes([head[2], head[3]]), 1);
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_to_compressed_cluster_decompresses_and_replaces() {
    let path = tmp_path("write_compressed");
    build_compressed_image(&path, 0xCC);

    // Original cluster reads as 0xCC throughout. After writing 0x11 bytes
    // at offset 100..116, the read should see 0xCC except at that slice.
    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(100, &[0x11u8; 16]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&path).unwrap();
    let mut window = [0u8; 32];
    r.read_at(90, &mut window).unwrap();
    // Bytes 90..100 untouched (10 bytes of 0xCC).
    assert_eq!(&window[0..10], &[0xCC; 10]);
    // Bytes 100..116 are the new payload.
    assert_eq!(&window[10..26], &[0x11; 16]);
    // Bytes 116..122 untouched (6 bytes of 0xCC).
    assert_eq!(&window[26..32], &[0xCC; 6]);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_to_ro_image_errors_with_read_only() {
    let path = tmp_path("write_ro");
    build_image(&path);

    let r = Qcow2Reader::open(&path).unwrap();
    assert!(!r.is_writable());
    match r.write_at(64, &[0x11; 4]) {
        Err(qcow2::Error::ReadOnly) => {}
        other => panic!("expected ReadOnly, got {other:?}"),
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn write_past_end_errors_with_out_of_bounds() {
    let path = tmp_path("write_oob");
    build_image(&path);

    let r = Qcow2Reader::open_rw(&path).unwrap();
    // VIRT_SIZE = 16384; write at the very end + 1.
    match r.write_at(VIRT_SIZE - 8, &[0x11u8; 16]) {
        Err(qcow2::Error::OutOfBounds { .. }) => {}
        other => panic!("expected OutOfBounds, got {other:?}"),
    }

    let _ = std::fs::remove_file(&path);
}

#[test]
fn open_rw_then_read_through_fs_core_blockdevice() {
    use fs_core::BlockDevice;

    let path = tmp_path("rw_fs_core");
    build_image(&path);

    let r = Qcow2Reader::open_rw(&path).unwrap();
    assert!(BlockDevice::is_writable(&r));

    // Trait surface should let us write + read identically to inherent.
    BlockDevice::write_at(&r, 100, &[0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    BlockDevice::flush(&r).unwrap();

    let mut buf = [0u8; 4];
    fs_core::BlockRead::read_at(&r, 100, &mut buf).unwrap();
    assert_eq!(buf, [0xDE, 0xAD, 0xBE, 0xEF]);

    let _ = std::fs::remove_file(&path);
}

// ---------------------------------------------------------------------------
// Phase 5b: snapshot-aware copy-on-write
//
// The check is per-cluster, not per-image: a write to a cluster whose host
// refcount > 1 must clone the cluster before mutating it, so the snapshot's
// view stays untouched. We simulate a snapshot by bumping the refcount of
// virt-cluster-0's host cluster to 2.
// ---------------------------------------------------------------------------

/// Bump the on-disk refcount of `host_cluster_idx` by `delta`. Used to
/// simulate the effect of taking an internal snapshot — every L2-referenced
/// cluster's refcount goes up by 1 when a snapshot lands.
fn bump_refcount(path: &PathBuf, refcount_block_off: u64, host_cluster_idx: usize, delta: u16) {
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let off = refcount_block_off + (host_cluster_idx as u64) * 2;
    f.seek(SeekFrom::Start(off)).unwrap();
    let mut cur = [0u8; 2];
    f.read_exact(&mut cur).unwrap();
    let new = u16::from_be_bytes(cur) + delta;
    f.seek(SeekFrom::Start(off)).unwrap();
    f.write_all(&new.to_be_bytes()).unwrap();
}

/// Clear the COPIED bit (bit 63) on virt cluster 0's L2 entry. qemu does
/// this for every L2 entry in the image when a snapshot lands, signalling
/// that the cluster is now shared and writes need CoW.
fn clear_copied_l2_entry_0(path: &PathBuf) {
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    f.seek(SeekFrom::Start(L2_OFFSET)).unwrap();
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).unwrap();
    let mut entry = u64::from_be_bytes(buf);
    entry &= !(1u64 << 63);
    f.seek(SeekFrom::Start(L2_OFFSET)).unwrap();
    f.write_all(&entry.to_be_bytes()).unwrap();
}

/// Read the L2 entry for virt cluster 0 from the synthetic image. Used to
/// confirm that the writer repointed L2 at a fresh host cluster (CoW)
/// rather than mutating the shared one.
fn read_l2_entry_0(path: &PathBuf) -> u64 {
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom};
    let mut f = OpenOptions::new().read(true).open(path).unwrap();
    f.seek(SeekFrom::Start(L2_OFFSET)).unwrap();
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf).unwrap();
    u64::from_be_bytes(buf)
}

/// Read a u16 refcount entry at the given host cluster index from the
/// synthetic refcount block.
fn read_refcount_entry(path: &PathBuf, refcount_block_off: u64, host_cluster_idx: u64) -> u16 {
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom};
    let mut f = OpenOptions::new().read(true).open(path).unwrap();
    let off = refcount_block_off + host_cluster_idx * 2;
    f.seek(SeekFrom::Start(off)).unwrap();
    let mut buf = [0u8; 2];
    f.read_exact(&mut buf).unwrap();
    u16::from_be_bytes(buf)
}

#[test]
fn write_to_shared_cluster_clones_via_cow() {
    // virt cluster 0 starts pointed at host cluster 3 (DATA0_OFFSET).
    // Bumping that host cluster's refcount to 2 simulates a snapshot
    // referencing the same data. The next write must NOT mutate cluster
    // 3 in place — it must allocate a fresh cluster, copy the existing
    // data into it, splice in the user payload, repoint L2 there, and
    // drop cluster 3's refcount back to 1.
    let path = tmp_path("cow_clone");
    build_image(&path);

    // Cluster 3 (DATA0_OFFSET / CLUSTER_SIZE) currently has refcount=1.
    // Bump to 2 and clear the L2 entry's COPIED bit to mimic the on-disk
    // state qemu produces when an internal snapshot lands.
    bump_refcount(&path, REFCOUNT_BLOCK_OFFSET, 3, 1);
    assert_eq!(read_refcount_entry(&path, REFCOUNT_BLOCK_OFFSET, 3), 2);
    clear_copied_l2_entry_0(&path);

    let original_l2 = read_l2_entry_0(&path);

    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(64, &[0x11, 0x22, 0x33, 0x44]).unwrap();
        r.flush().unwrap();
    }

    // L2 entry must now point at a NEW host cluster, not cluster 3.
    let new_l2 = read_l2_entry_0(&path);
    assert_ne!(
        new_l2 & 0x00ff_ffff_ffff_fe00,
        original_l2 & 0x00ff_ffff_ffff_fe00,
        "L2 entry must be repointed at a fresh cluster after CoW"
    );

    // Old cluster's refcount drops back to 1 (the snapshot still holds it).
    assert_eq!(
        read_refcount_entry(&path, REFCOUNT_BLOCK_OFFSET, 3),
        1,
        "old cluster's refcount must drop by 1 after CoW"
    );

    // The new cluster's refcount should be 1.
    let new_host_cluster_idx = (new_l2 & 0x00ff_ffff_ffff_fe00) / CLUSTER_SIZE;
    assert_eq!(
        read_refcount_entry(&path, REFCOUNT_BLOCK_OFFSET, new_host_cluster_idx),
        1,
        "freshly-allocated CoW cluster must have refcount=1"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn snapshot_view_unchanged_after_cow_write() {
    // After the write CoW's the cluster, an outsider reading the OLD
    // host cluster directly (the snapshot's view) must still see the
    // original 0xAA bytes — the writer must not have touched it.
    let path = tmp_path("cow_snap_view");
    build_image(&path);
    bump_refcount(&path, REFCOUNT_BLOCK_OFFSET, 3, 1);
    clear_copied_l2_entry_0(&path);

    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(64, &[0x11, 0x22, 0x33, 0x44]).unwrap();
        r.flush().unwrap();
    }

    // Read the original host cluster directly off disk (DATA0_OFFSET).
    use std::fs::OpenOptions;
    use std::io::{Read, Seek, SeekFrom};
    let mut f = OpenOptions::new().read(true).open(&path).unwrap();
    f.seek(SeekFrom::Start(DATA0_OFFSET)).unwrap();
    let mut buf = vec![0u8; CLUSTER_SIZE as usize];
    f.read_exact(&mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xAA),
        "snapshot's host cluster must remain untouched after CoW write"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn live_view_sees_cow_write() {
    // Mirror of the previous test: the live image's view at virt 0 must
    // reflect the new bytes (CoW means the writer points L2 at a fresh
    // cluster carrying the spliced data).
    let path = tmp_path("cow_live_view");
    build_image(&path);
    bump_refcount(&path, REFCOUNT_BLOCK_OFFSET, 3, 1);
    clear_copied_l2_entry_0(&path);

    {
        let r = Qcow2Reader::open_rw(&path).unwrap();
        r.write_at(64, &[0x11, 0x22, 0x33, 0x44]).unwrap();
        r.flush().unwrap();
    }

    let r = Qcow2Reader::open(&path).unwrap();
    let mut buf = [0u8; 8];
    r.read_at(60, &mut buf).unwrap();
    assert_eq!(buf, [0xAA, 0xAA, 0xAA, 0xAA, 0x11, 0x22, 0x33, 0x44]);

    // Bytes outside the spliced range stay 0xAA — proves the rest of
    // the cluster was copied verbatim from the original.
    let mut tail = [0u8; 4];
    r.read_at(72, &mut tail).unwrap();
    assert_eq!(tail, [0xAA; 4]);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn zstd_compressed_cluster_round_trip() {
    let path = tmp_path("zstd_compressed");
    build_zstd_compressed_image(&path, 0xCC);

    let r = Qcow2Reader::open(&path).unwrap();
    assert_eq!(r.header().compression_type, 1);

    let mut buf = vec![0u8; 4096];
    r.read_at(0, &mut buf).unwrap();
    assert!(
        buf.iter().all(|&b| b == 0xCC),
        "zstd-decompressed cluster must match input pattern"
    );

    // Cache hit: read again, smaller window.
    let mut buf2 = vec![0u8; 16];
    r.read_at(2048, &mut buf2).unwrap();
    assert!(buf2.iter().all(|&b| b == 0xCC));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn cluster_status_classifies_each_virtual_cluster() {
    use qcow2::ClusterStatus;
    let path = tmp_path("cluster_status");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    // Layout: virt 0 = allocated, virt 1 = unallocated, virt 2 = allocated, virt 3 = zero.
    assert_eq!(r.cluster_status_at(0).unwrap(), ClusterStatus::Allocated);
    assert_eq!(
        r.cluster_status_at(CLUSTER_SIZE).unwrap(),
        ClusterStatus::Unallocated
    );
    assert_eq!(
        r.cluster_status_at(CLUSTER_SIZE * 2).unwrap(),
        ClusterStatus::Allocated
    );
    assert_eq!(
        r.cluster_status_at(CLUSTER_SIZE * 3).unwrap(),
        ClusterStatus::Zero
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn cluster_status_rejects_out_of_bounds_offset() {
    let path = tmp_path("cluster_status_oob");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    assert!(r.cluster_status_at(VIRT_SIZE).is_err());
    assert!(r.cluster_status_at(VIRT_SIZE * 2).is_err());

    let _ = std::fs::remove_file(&path);
}

#[test]
fn extents_iter_yields_run_length_encoded_status_runs() {
    use qcow2::ClusterStatus;
    let path = tmp_path("extents");
    build_image(&path);
    let r = Qcow2Reader::open(&path).unwrap();

    let extents: Vec<_> = r.extents().collect::<qcow2::Result<Vec<_>>>().unwrap();

    // Expected, from the layout comment at the top of the file:
    //   virt 0 = Allocated, virt 1 = Unallocated,
    //   virt 2 = Allocated, virt 3 = Zero.
    // No two adjacent clusters share a status, so each cluster is its
    // own extent: 4 extents total, each 1 cluster long.
    assert_eq!(extents.len(), 4);

    assert_eq!(extents[0].virt_offset, 0);
    assert_eq!(extents[0].length, CLUSTER_SIZE);
    assert_eq!(extents[0].status, ClusterStatus::Allocated);

    assert_eq!(extents[1].virt_offset, CLUSTER_SIZE);
    assert_eq!(extents[1].length, CLUSTER_SIZE);
    assert_eq!(extents[1].status, ClusterStatus::Unallocated);

    assert_eq!(extents[2].virt_offset, CLUSTER_SIZE * 2);
    assert_eq!(extents[2].length, CLUSTER_SIZE);
    assert_eq!(extents[2].status, ClusterStatus::Allocated);

    assert_eq!(extents[3].virt_offset, CLUSTER_SIZE * 3);
    assert_eq!(extents[3].length, CLUSTER_SIZE);
    assert_eq!(extents[3].status, ClusterStatus::Zero);

    // Lengths sum to virtual size.
    let total: u64 = extents.iter().map(|e| e.length).sum();
    assert_eq!(total, VIRT_SIZE);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn extents_iter_handles_freshly_created_all_unallocated_image() {
    use qcow2::ClusterStatus;
    use std::process::Command;
    if Command::new("qemu-img").arg("--version").output().is_err() {
        return;
    }
    let p = tmp_path("freshly_created");
    let status = Command::new("qemu-img")
        .args(["create", "-f", "qcow2", p.to_str().unwrap(), "1M"])
        .status()
        .unwrap();
    assert!(status.success());

    let r = Qcow2Reader::open(&p).unwrap();
    let extents: Vec<_> = r.extents().collect::<qcow2::Result<Vec<_>>>().unwrap();

    // Brand-new qcow2 has no allocated clusters at all — the iterator
    // collapses the whole virtual disk into one Unallocated extent.
    assert_eq!(extents.len(), 1);
    assert_eq!(extents[0].virt_offset, 0);
    assert_eq!(extents[0].length, r.virtual_size());
    assert_eq!(extents[0].status, ClusterStatus::Unallocated);

    let _ = std::fs::remove_file(&p);
}

/// Patch `len` big-endian bytes of `val` into `path` at byte `off`.
fn patch_be(path: &PathBuf, off: u64, val: u64, len: usize) {
    use std::io::{Seek, SeekFrom, Write};
    debug_assert!(len <= 8, "patch_be len must be <= 8, got {len}");
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(off)).unwrap();
    f.write_all(&val.to_be_bytes()[8 - len..]).unwrap();
    f.flush().unwrap();
}

/// open() must enforce Header::check_supported — build a spec-valid image
/// and flip crypt_method to AES; the reader must refuse it rather than
/// hand back a handle that would read ciphertext as plaintext.
#[test]
fn open_rejects_encrypted_image() {
    let p = tmp_path("encrypted");
    build_image(&p);
    patch_be(&p, 32, 1, 4); // crypt_method = AES at header offset 32
    match Qcow2Reader::open(&p).err() {
        Some(qcow2::Error::Unsupported(m)) => assert!(m.contains("encryption")),
        other => panic!("expected Unsupported(encryption), got {other:?}"),
    }
    let _ = std::fs::remove_file(&p);
}

/// open() must also refuse an image declaring the external-data-file
/// incompatible feature.
#[test]
fn open_rejects_external_data_file_image() {
    let p = tmp_path("data-file");
    build_image(&p);
    // incompatible_features at header offset 72; bit 2 = DATA_FILE.
    patch_be(&p, 72, 1 << 2, 8);
    match Qcow2Reader::open(&p).err() {
        Some(qcow2::Error::Unsupported(m)) => assert_eq!(m, "external data file"),
        other => panic!("expected Unsupported(external data file), got {other:?}"),
    }
    let _ = std::fs::remove_file(&p);
}

/// A malformed image whose refcount block marks host cluster 0 free must
/// not be handed that cluster by the allocator.
///
/// Cluster 0 holds the header. `allocate_cluster` scans from block 0,
/// entry 0, so a refcount of zero there made it the first free cluster
/// found — and the caller immediately zero-fills what it is given, which
/// wipes the header. The L2 entry written afterwards is `0 | COPIED`,
/// which `lookup_cluster` reads straight back as `Unallocated`, so the
/// write is lost as well.
///
/// Every well-formed image marks cluster 0 in use, which is why no
/// fixture reached this and why it has to be built deliberately.
#[test]
fn allocator_refuses_the_header_cluster() {
    let path = tmp_path("refcount_zero_cluster0");
    build_image(&path);

    // Clear the refcount for host cluster 0 only, leaving 1..=6 in use.
    // `WriteAt` is the portable seek-then-write in `tests/common`; the
    // positional syscalls it stands in for are Unix-only, and CI runs
    // this on Windows too.
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.write_all_at(&0u16.to_be_bytes(), REFCOUNT_BLOCK_OFFSET)
            .unwrap();
    }

    let r = Qcow2Reader::open_rw(&path).unwrap();

    // Writing into an unallocated virtual cluster forces an allocation.
    let err = r
        .write_at(CLUSTER_SIZE * 3, &[0xEEu8; 512])
        .expect_err("the allocator must not hand out the header cluster");
    let msg = format!("{err}");
    assert!(
        msg.contains("cluster 0"),
        "the refusal should name the cluster, got: {msg}"
    );

    // And the header survived.
    drop(r);
    let mut magic = [0u8; 4];
    let mut hf = std::fs::File::open(&path).unwrap();
    hf.read_exact_at(&mut magic, 0).unwrap();
    assert_eq!(&magic, b"QFI\xfb", "the header was overwritten");

    let _ = std::fs::remove_file(&path);
}
