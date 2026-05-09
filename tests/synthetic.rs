//! End-to-end test on a hand-crafted qcow2 v3 image.
//!
//! Layout (cluster_size = 4096):
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

use qcow2::Qcow2Reader;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

const QCOW2_MAGIC: u32 = 0x5146_49fb;
const CLUSTER_SIZE: u64 = 4096;
const VIRT_SIZE: u64 = CLUSTER_SIZE * 4;

const L1_OFFSET: u64 = CLUSTER_SIZE;
const L2_OFFSET: u64 = CLUSTER_SIZE * 2;
const DATA0_OFFSET: u64 = CLUSTER_SIZE * 3;
const DATA2_OFFSET: u64 = CLUSTER_SIZE * 4;
const REFCOUNT_TABLE_OFFSET: u64 = CLUSTER_SIZE * 5;
const REFCOUNT_BLOCK_OFFSET: u64 = CLUSTER_SIZE * 6;
/// 16 clusters total: 7 in-use (header through refcount block) + 9 free
/// host clusters available for Phase B allocation. Refcount block has
/// room for 2048 entries so the rest stays available too.
const TOTAL_CLUSTERS: u64 = 16;
const TOTAL_SIZE: u64 = CLUSTER_SIZE * TOTAL_CLUSTERS;

const COPIED: u64 = 1u64 << 63;
const L2_ZERO: u64 = 1u64 << 0;

fn build_image(path: &PathBuf) {
    let mut f = File::create(path).unwrap();
    f.set_len(TOTAL_SIZE).unwrap();

    // ---- cluster 0: header ----
    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes()); // version 3
    hdr[8..16].copy_from_slice(&0u64.to_be_bytes()); // backing_file_offset
    hdr[16..20].copy_from_slice(&0u32.to_be_bytes()); // backing_file_size
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes()); // cluster_bits
    hdr[24..32].copy_from_slice(&VIRT_SIZE.to_be_bytes()); // size
    hdr[32..36].copy_from_slice(&0u32.to_be_bytes()); // crypt
    hdr[36..40].copy_from_slice(&1u32.to_be_bytes()); // l1_size
    hdr[40..48].copy_from_slice(&L1_OFFSET.to_be_bytes()); // l1_table_offset
    hdr[48..56].copy_from_slice(&REFCOUNT_TABLE_OFFSET.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes()); // refcount_table_clusters
    hdr[60..64].copy_from_slice(&0u32.to_be_bytes()); // nb_snapshots
    hdr[64..72].copy_from_slice(&0u64.to_be_bytes()); // snapshots_offset
    hdr[72..80].copy_from_slice(&0u64.to_be_bytes()); // incompatible_features
    hdr[80..88].copy_from_slice(&0u64.to_be_bytes()); // compatible_features
    hdr[88..96].copy_from_slice(&0u64.to_be_bytes()); // autoclear_features
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes()); // refcount_order (=> u16 refcounts)
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes()); // header_length
    f.write_all_at(&hdr, 0).unwrap();

    // ---- cluster 1: L1 table ----
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // ---- cluster 2: L2 table ----
    let mut l2 = [0u8; 4096];
    let e0 = (DATA0_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    let e2 = (DATA2_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    let e3 = COPIED | L2_ZERO;
    l2[0..8].copy_from_slice(&e0.to_be_bytes());
    // l2[8..16] left zero (unallocated)
    l2[16..24].copy_from_slice(&e2.to_be_bytes());
    l2[24..32].copy_from_slice(&e3.to_be_bytes());
    f.write_all_at(&l2, L2_OFFSET).unwrap();

    // ---- data clusters ----
    let mut d0 = [0u8; 4096];
    d0.fill(0xAA);
    f.write_all_at(&d0, DATA0_OFFSET).unwrap();

    let mut d2 = [0u8; 4096];
    d2.fill(0xBB);
    f.write_all_at(&d2, DATA2_OFFSET).unwrap();

    // ---- cluster 5: refcount table (1 entry pointing at the block) ----
    let mut rt = [0u8; 4096];
    rt[0..8].copy_from_slice(&REFCOUNT_BLOCK_OFFSET.to_be_bytes());
    f.write_all_at(&rt, REFCOUNT_TABLE_OFFSET).unwrap();

    // ---- cluster 6: refcount block ----
    // u16 entries, big-endian. Clusters 0..=6 are in use (refcount=1);
    // clusters 7..2047 are free (refcount=0).
    let mut rb = [0u8; 4096];
    for cluster_idx in 0..7u16 {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, REFCOUNT_BLOCK_OFFSET).unwrap();
}

// Tiny trait-shim so we can write at offsets without juggling Seek state.
trait WriteAt {
    fn write_all_at(&mut self, buf: &[u8], offset: u64) -> std::io::Result<()>;
}
impl WriteAt for File {
    fn write_all_at(&mut self, buf: &[u8], offset: u64) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(buf)
    }
}

fn tmp_path(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "qcow2_synth_{}_{}_{name}.qcow2",
        std::process::id(),
        n
    ));
    p
}

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

// ---------------------------------------------------------------------------
// Phase 2: compression + backing chain
// ---------------------------------------------------------------------------

const L2_FLAG_COMPRESSED: u64 = 1u64 << 62;
const L2_FLAG_ZERO: u64 = 1u64 << 0;

/// Build a qcow2 v3 image whose virt cluster 0 holds a *compressed* cluster
/// of `pattern`-filled bytes; other virt clusters are unallocated.
///
/// Layout:
///   cluster 0   header
///   cluster 1   L1 table
///   cluster 2   L2 table
///   cluster 3+  compressed bytes (sector-padded)
fn build_compressed_image(path: &PathBuf, pattern: u8) {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;

    // Compress 4096 bytes of `pattern` with raw deflate.
    let plain = vec![pattern; CLUSTER_SIZE as usize];
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&plain).unwrap();
    let compressed = enc.finish().unwrap();
    assert!(
        compressed.len() < CLUSTER_SIZE as usize,
        "compressed payload should be smaller than a cluster"
    );

    // Place compressed bytes starting at the host byte offset (cluster 3, offset 0).
    let comp_host_off: u64 = CLUSTER_SIZE * 3;
    // Span in 512-byte sectors. n = (sectors_used - 1).
    let span_bytes = compressed.len().div_ceil(512) * 512;
    let n_sectors_minus1 = ((span_bytes / 512) - 1) as u64;

    // Encode the compressed-cluster descriptor.
    // x = 62 - (cluster_bits - 8). For cluster_bits=12, x=58.
    let x: u64 = 62 - (12 - 8);
    let descriptor = comp_host_off | (n_sectors_minus1 << x);
    let l2_entry_compressed = COPIED | L2_FLAG_COMPRESSED | descriptor;

    // Layout: header(0) + L1(1) + L2(2) + compressed(3..3+span_clusters) +
    //         refcount table(N) + refcount block(N+1) + free space.
    let span_clusters = (span_bytes.div_ceil(CLUSTER_SIZE as usize) as u64).max(1);
    let comp_end_cluster = 3 + span_clusters; // first cluster after compressed
    let rt_cluster = comp_end_cluster;
    let rb_cluster = comp_end_cluster + 1;
    let total_clusters = (rb_cluster + 8).max(16);
    let total = CLUSTER_SIZE * total_clusters;
    let mut f = File::create(path).unwrap();
    f.set_len(total).unwrap();

    // Header.
    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
    hdr[24..32].copy_from_slice(&VIRT_SIZE.to_be_bytes());
    hdr[36..40].copy_from_slice(&1u32.to_be_bytes());
    hdr[40..48].copy_from_slice(&L1_OFFSET.to_be_bytes());
    let rt_off = rt_cluster * CLUSTER_SIZE;
    hdr[48..56].copy_from_slice(&rt_off.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes()); // refcount_table_clusters
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
    f.write_all_at(&hdr, 0).unwrap();

    // L1 table → L2 cluster.
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // L2 table — entry 0 is compressed, the rest unallocated.
    let mut l2 = [0u8; 4096];
    l2[0..8].copy_from_slice(&l2_entry_compressed.to_be_bytes());
    f.write_all_at(&l2, L2_OFFSET).unwrap();

    // Compressed bytes (zero-padded to span_bytes).
    let mut sector_buf = vec![0u8; span_bytes];
    sector_buf[..compressed.len()].copy_from_slice(&compressed);
    f.write_all_at(&sector_buf, comp_host_off).unwrap();

    // Refcount table (one entry pointing at the block).
    let mut rt = [0u8; 4096];
    let rb_off = rb_cluster * CLUSTER_SIZE;
    rt[0..8].copy_from_slice(&rb_off.to_be_bytes());
    f.write_all_at(&rt, rt_off).unwrap();

    // Refcount block: clusters 0..=rb_cluster are in use; rest free.
    let mut rb = [0u8; 4096];
    for cluster_idx in 0..=rb_cluster {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    // The compressed payload also occupies span_clusters host clusters
    // starting at cluster 3 — already covered by the loop above when
    // span=1; for span=2 they're cluster 3 and 4. Loop already includes
    // them since rb_cluster = 3 + span.
    f.write_all_at(&rb, rb_off).unwrap();
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

// ---------------------------------------------------------------------------
// Backing chain
// ---------------------------------------------------------------------------

/// Build a child qcow2 whose L1 has one allocated L2 table; entries are
/// caller-provided so we can mix unallocated and zero-flagged. The child also
/// sets a backing file pointing at `backing_path` (relative).
fn build_child_with_backing(
    path: &PathBuf,
    backing_relative_name: &str,
    l2_entries: &[(usize, u64)],
) {
    let mut f = File::create(path).unwrap();
    f.set_len(CLUSTER_SIZE * 3).unwrap();

    let backing_path_offset: u64 = 0x200;
    let backing_bytes = backing_relative_name.as_bytes();

    // Header.
    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
    hdr[8..16].copy_from_slice(&backing_path_offset.to_be_bytes());
    hdr[16..20].copy_from_slice(&(backing_bytes.len() as u32).to_be_bytes());
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
    hdr[24..32].copy_from_slice(&VIRT_SIZE.to_be_bytes());
    hdr[36..40].copy_from_slice(&1u32.to_be_bytes());
    hdr[40..48].copy_from_slice(&L1_OFFSET.to_be_bytes());
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
    // Backing path string lives in the header cluster at offset 0x200.
    hdr[backing_path_offset as usize..backing_path_offset as usize + backing_bytes.len()]
        .copy_from_slice(backing_bytes);
    f.write_all_at(&hdr, 0).unwrap();

    // L1.
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // L2.
    let mut l2 = [0u8; 4096];
    for (idx, val) in l2_entries {
        let off = idx * 8;
        l2[off..off + 8].copy_from_slice(&val.to_be_bytes());
    }
    f.write_all_at(&l2, L2_OFFSET).unwrap();
}

fn pair_paths(name: &str) -> (PathBuf, PathBuf, String) {
    let dir = std::env::temp_dir();
    let stamp = format!(
        "{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let parent = dir.join(format!("qcow2_parent_{stamp}.qcow2"));
    let child = dir.join(format!("qcow2_child_{stamp}.qcow2"));
    let rel = parent.file_name().unwrap().to_string_lossy().into_owned();
    (parent, child, rel)
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

// Reference Path so test layout stays explicit even though we don't use it.
#[allow(dead_code)]
fn _path_helper(p: &Path) -> &Path {
    p
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

// ---------------------------------------------------------------------------
// Phase 5c: zstd-compressed clusters
// ---------------------------------------------------------------------------

/// Bit 3 of `incompatible_features` — the spec mandates it when
/// `compression_type != 0`.
const INCOMPAT_COMPRESSION_TYPE: u64 = 1 << 3;

/// Build a qcow2 v3 image whose virt cluster 0 holds a *zstd-compressed*
/// cluster of `pattern`-filled bytes. The header sets compression_type=1
/// at byte 104 and the matching incompatible-features bit. Layout mirrors
/// `build_compressed_image`.
fn build_zstd_compressed_image(path: &PathBuf, pattern: u8) {
    // Compress 4096 bytes of `pattern` with zstd. Default level is fine —
    // payload pattern is highly compressible.
    let plain = vec![pattern; CLUSTER_SIZE as usize];
    let compressed = zstd::stream::encode_all(&plain[..], 0).unwrap();
    assert!(
        compressed.len() < CLUSTER_SIZE as usize,
        "compressed payload should be smaller than a cluster"
    );

    let comp_host_off: u64 = CLUSTER_SIZE * 3;
    let span_bytes = compressed.len().div_ceil(512) * 512;
    let n_sectors_minus1 = ((span_bytes / 512) - 1) as u64;

    let x: u64 = 62 - (12 - 8);
    let descriptor = comp_host_off | (n_sectors_minus1 << x);
    let l2_entry_compressed = COPIED | L2_FLAG_COMPRESSED | descriptor;

    let span_clusters = (span_bytes.div_ceil(CLUSTER_SIZE as usize) as u64).max(1);
    let comp_end_cluster = 3 + span_clusters;
    let rt_cluster = comp_end_cluster;
    let rb_cluster = comp_end_cluster + 1;
    let total_clusters = (rb_cluster + 8).max(16);
    let total = CLUSTER_SIZE * total_clusters;
    let mut f = File::create(path).unwrap();
    f.set_len(total).unwrap();

    // Header — note compression_type=1 at byte 104, header_length=112,
    // and the incompatible-features bit set.
    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
    hdr[24..32].copy_from_slice(&VIRT_SIZE.to_be_bytes());
    hdr[36..40].copy_from_slice(&1u32.to_be_bytes());
    hdr[40..48].copy_from_slice(&L1_OFFSET.to_be_bytes());
    let rt_off = rt_cluster * CLUSTER_SIZE;
    hdr[48..56].copy_from_slice(&rt_off.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes());
    hdr[72..80].copy_from_slice(&INCOMPAT_COMPRESSION_TYPE.to_be_bytes());
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&112u32.to_be_bytes()); // header_length=112
    hdr[104] = 1; // compression_type = zstd
    f.write_all_at(&hdr, 0).unwrap();

    // L1 → L2.
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & 0x00ff_ffff_ffff_fe00) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // L2 — entry 0 is compressed, rest unallocated.
    let mut l2 = [0u8; 4096];
    l2[0..8].copy_from_slice(&l2_entry_compressed.to_be_bytes());
    f.write_all_at(&l2, L2_OFFSET).unwrap();

    // Compressed bytes (zero-padded to span_bytes).
    let mut sector_buf = vec![0u8; span_bytes];
    sector_buf[..compressed.len()].copy_from_slice(&compressed);
    f.write_all_at(&sector_buf, comp_host_off).unwrap();

    // Refcount table.
    let mut rt = [0u8; 4096];
    let rb_off = rb_cluster * CLUSTER_SIZE;
    rt[0..8].copy_from_slice(&rb_off.to_be_bytes());
    f.write_all_at(&rt, rt_off).unwrap();

    // Refcount block.
    let mut rb = [0u8; 4096];
    for cluster_idx in 0..=rb_cluster {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, rb_off).unwrap();
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
