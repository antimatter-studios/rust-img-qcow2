//! Shared synthetic-image builders for integration tests.
//!
//! Each integration test binary in `tests/` (`synthetic.rs`,
//! `qemu_validation.rs`, …) can include this module via `mod common;`.
//! Builders here hand-write the on-disk byte layout per the qcow2 spec —
//! the reader/writer is never invoked when producing fixtures.
//!
//! Every builder produces an image that is intended to be **spec-valid**:
//! header, L1, L2, refcount table, and refcount block are all present
//! and consistent. `tests/qemu_validation.rs` confirms this externally
//! via `qemu-img check`.

#![allow(dead_code)] // not every consumer uses every helper

use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const QCOW2_MAGIC: u32 = 0x5146_49fb;
pub const CLUSTER_SIZE: u64 = 4096;
pub const VIRT_SIZE: u64 = CLUSTER_SIZE * 4;

pub const L1_OFFSET: u64 = CLUSTER_SIZE;
pub const L2_OFFSET: u64 = CLUSTER_SIZE * 2;
pub const DATA0_OFFSET: u64 = CLUSTER_SIZE * 3;
pub const DATA2_OFFSET: u64 = CLUSTER_SIZE * 4;
pub const REFCOUNT_TABLE_OFFSET: u64 = CLUSTER_SIZE * 5;
pub const REFCOUNT_BLOCK_OFFSET: u64 = CLUSTER_SIZE * 6;

/// 16 clusters total: 7 in-use (header through refcount block) + 9 free
/// host clusters available for Phase B allocation.
pub const TOTAL_CLUSTERS: u64 = 16;
pub const TOTAL_SIZE: u64 = CLUSTER_SIZE * TOTAL_CLUSTERS;

pub const COPIED: u64 = 1u64 << 63;
pub const L2_ZERO: u64 = 1u64 << 0;
pub const L2_FLAG_ZERO: u64 = L2_ZERO;
pub const L2_FLAG_COMPRESSED: u64 = 1u64 << 62;

/// Bits 9..55 — the host-offset field of an L1 or uncompressed L2
/// entry.
///
/// Written out longhand at eleven sites before this. It is defined here
/// rather than imported from the crate on purpose: the fixtures are
/// meant to be an independent statement of the format, so a wrong mask
/// in `reader.rs` shows up as a failure instead of being agreed with.
/// One copy per side, not eleven on one.
pub const HOST_OFFSET_MASK: u64 = 0x00ff_ffff_ffff_fe00;

/// Bit 3 of `incompatible_features` — the spec mandates it when
/// `compression_type != 0`.
pub const INCOMPAT_COMPRESSION_TYPE: u64 = 1 << 3;

/// Positional I/O that works on every target.
///
/// `std::os::unix::fs::FileExt` has `write_all_at` and `read_exact_at`,
/// and does not exist on Windows — where CI also runs. Seek-then-read
/// is the portable equivalent, which is why these take `&mut self`: the
/// file cursor moves, so the same handle cannot be shared across
/// threads the way the positional syscalls can.
pub trait WriteAt {
    fn write_all_at(&mut self, buf: &[u8], offset: u64) -> std::io::Result<()>;
}
impl WriteAt for File {
    fn write_all_at(&mut self, buf: &[u8], offset: u64) -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        self.seek(SeekFrom::Start(offset))?;
        self.write_all(buf)
    }
}

/// The reading half of [`WriteAt`], for the same reason.
pub trait ReadAt {
    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> std::io::Result<()>;
}
impl ReadAt for File {
    fn read_exact_at(&mut self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        self.seek(SeekFrom::Start(offset))?;
        self.read_exact(buf)
    }
}

pub fn tmp_path(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "qcow2_test_{}_{}_{name}.qcow2",
        std::process::id(),
        n
    ));
    p
}

/// Build a pair of related image paths plus the backing-relative name
/// used to wire a child to its parent. Returns
/// `(parent_path, child_path, parent_basename)`.
pub fn pair_paths(name: &str) -> (PathBuf, PathBuf, String) {
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

/// Build a four-virt-cluster qcow2 v3 image:
///
/// ```text
///   virt 0 -> data cluster filled with 0xAA  (allocated)
///   virt 1 -> unallocated                    (reads zeros)
///   virt 2 -> data cluster filled with 0xBB  (allocated)
///   virt 3 -> v3 zero flag                   (reads zeros)
/// ```
pub fn build_image(path: &Path) {
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
    let l1_entry = (L2_OFFSET & HOST_OFFSET_MASK) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // ---- cluster 2: L2 table ----
    let mut l2 = [0u8; 4096];
    let e0 = (DATA0_OFFSET & HOST_OFFSET_MASK) | COPIED;
    let e2 = (DATA2_OFFSET & HOST_OFFSET_MASK) | COPIED;
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

/// Which compressor a compressed-cluster fixture uses.
///
/// The zlib and zstd builders were ~70 lines each and differed in the
/// compressor call and three header bytes. Everything else — the
/// descriptor encoding, the layout arithmetic, the L1/L2 writes, the
/// refcount table and block — was byte-for-byte the same, which meant a
/// fix to the shared 90% had to be made twice and could be made once.
#[derive(Clone, Copy)]
pub enum Compressor {
    /// qcow2 `compression_type = 0`: raw deflate.
    Zlib,
    /// qcow2 `compression_type = 1`, plus the matching
    /// incompatible-features bit.
    Zstd,
}

/// Build a qcow2 v3 image whose virt cluster 0 holds a *compressed*
/// cluster of `pattern`-filled bytes; other virt clusters are
/// unallocated.
pub fn build_compressed_image(path: &Path, pattern: u8) {
    build_compressed_image_with(path, pattern, Compressor::Zlib);
}

/// As [`build_compressed_image`], with zstd. The header sets
/// `compression_type = 1` at byte 104 and the matching
/// incompatible-features bit.
pub fn build_zstd_compressed_image(path: &Path, pattern: u8) {
    build_compressed_image_with(path, pattern, Compressor::Zstd);
}

/// The one compressed-cluster builder.
///
/// Offsets are still written as literals rather than through
/// `qcow2::header::offsets` — deliberately, so this fixture is an
/// independent statement of the layout and can disagree with the
/// parser. See that module's docs.
pub fn build_compressed_image_with(path: &Path, pattern: u8, compressor: Compressor) {
    let plain = vec![pattern; CLUSTER_SIZE as usize];
    build_compressed_image_of(path, &plain, compressor);
}

/// As [`build_compressed_image_with`], with the plaintext given rather
/// than a cluster of one repeated byte.
///
/// A cluster's descriptor says how many 512-byte sectors the compressed
/// payload occupies; it says nothing about how much it decodes to. So a
/// payload that decodes to far more than a cluster is a legal thing to
/// write down, and this is how a test writes one.
pub fn build_compressed_image_of(path: &Path, plain: &[u8], compressor: Compressor) {
    let compressed = match compressor {
        Compressor::Zlib => {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
            enc.write_all(plain).unwrap();
            enc.finish().unwrap()
        }
        Compressor::Zstd => zstd::stream::encode_all(plain, 0).unwrap(),
    };
    assert!(
        compressed.len() < CLUSTER_SIZE as usize,
        "compressed payload should be smaller than a cluster"
    );

    let comp_host_off: u64 = CLUSTER_SIZE * 3;
    let span_bytes = compressed.len().div_ceil(512) * 512;
    let n_sectors_minus1 = ((span_bytes / 512) - 1) as u64;

    // Encode the compressed-cluster descriptor.
    // x = 62 - (cluster_bits - 8). For cluster_bits=12, x=58.
    let x: u64 = 62 - (12 - 8);
    let descriptor = comp_host_off | (n_sectors_minus1 << x);
    // Per spec: COPIED (bit 63) MUST NOT be set on compressed L2
    // entries; the COMPRESSED flag itself is the indicator. qemu-img
    // check rejects an image that sets both.
    let l2_entry_compressed = L2_FLAG_COMPRESSED | descriptor;

    let span_clusters = (span_bytes.div_ceil(CLUSTER_SIZE as usize) as u64).max(1);
    let comp_end_cluster = 3 + span_clusters;
    let rt_cluster = comp_end_cluster;
    let rb_cluster = comp_end_cluster + 1;
    let total_clusters = (rb_cluster + 8).max(16);
    let total = CLUSTER_SIZE * total_clusters;
    let mut f = File::create(path).unwrap();
    f.set_len(total).unwrap();

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
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    // The three bytes that are the whole difference on disk.
    match compressor {
        Compressor::Zlib => {
            hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
        }
        Compressor::Zstd => {
            hdr[72..80].copy_from_slice(&INCOMPAT_COMPRESSION_TYPE.to_be_bytes());
            hdr[100..104].copy_from_slice(&112u32.to_be_bytes());
            hdr[104] = 1; // compression_type = zstd
        }
    }
    f.write_all_at(&hdr, 0).unwrap();

    // L1 → L2.
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & HOST_OFFSET_MASK) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // L2 — entry 0 compressed, rest unallocated.
    let mut l2 = [0u8; 4096];
    l2[0..8].copy_from_slice(&l2_entry_compressed.to_be_bytes());
    f.write_all_at(&l2, L2_OFFSET).unwrap();

    // Compressed bytes, zero-padded to span_bytes.
    let mut sector_buf = vec![0u8; span_bytes];
    sector_buf[..compressed.len()].copy_from_slice(&compressed);
    f.write_all_at(&sector_buf, comp_host_off).unwrap();

    // Refcount table.
    let mut rt = [0u8; 4096];
    let rb_off = rb_cluster * CLUSTER_SIZE;
    rt[0..8].copy_from_slice(&rb_off.to_be_bytes());
    f.write_all_at(&rt, rt_off).unwrap();

    // Refcount block: clusters 0..=rb_cluster in use.
    let mut rb = [0u8; 4096];
    for cluster_idx in 0..=rb_cluster {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, rb_off).unwrap();
}

/// Build a child qcow2 with a backing-file pointer and a caller-provided
/// L2-entry list. The child stores its backing-path string in the header
/// cluster at byte offset 0x200. **Includes refcount metadata** so the
/// resulting image is structurally valid by itself, not just by the
/// reader's view.
///
/// Layout:
///
/// ```text
///   cluster 0   header (with backing path string at byte 0x200)
///   cluster 1   L1 table
///   cluster 2   L2 table
///   cluster 3   refcount table
///   cluster 4   refcount block
/// ```
pub fn build_child_with_backing(
    path: &Path,
    backing_relative_name: &str,
    l2_entries: &[(usize, u64)],
) {
    let rt_cluster: u64 = 3;
    let rb_cluster: u64 = 4;
    let total_clusters: u64 = 5;

    let mut f = File::create(path).unwrap();
    f.set_len(CLUSTER_SIZE * total_clusters).unwrap();

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
    let rt_off = rt_cluster * CLUSTER_SIZE;
    hdr[48..56].copy_from_slice(&rt_off.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes());
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
    hdr[backing_path_offset as usize..backing_path_offset as usize + backing_bytes.len()]
        .copy_from_slice(backing_bytes);
    f.write_all_at(&hdr, 0).unwrap();

    // L1.
    let mut l1 = [0u8; 4096];
    let l1_entry = (L2_OFFSET & HOST_OFFSET_MASK) | COPIED;
    l1[0..8].copy_from_slice(&l1_entry.to_be_bytes());
    f.write_all_at(&l1, L1_OFFSET).unwrap();

    // L2 — caller-provided entries.
    let mut l2 = [0u8; 4096];
    for (idx, val) in l2_entries {
        let off = idx * 8;
        l2[off..off + 8].copy_from_slice(&val.to_be_bytes());
    }
    f.write_all_at(&l2, L2_OFFSET).unwrap();

    // Refcount table.
    let mut rt = [0u8; 4096];
    let rb_off = rb_cluster * CLUSTER_SIZE;
    rt[0..8].copy_from_slice(&rb_off.to_be_bytes());
    f.write_all_at(&rt, rt_off).unwrap();

    // Refcount block: clusters 0..total_clusters in use.
    let mut rb = [0u8; 4096];
    for cluster_idx in 0..total_clusters as u16 {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, rb_off).unwrap();
}

/// Build an image whose virtual disk needs **more than one L1 entry**.
///
/// One L1 entry covers `cluster_size / 8` clusters — 512 of them at a
/// 4 KiB cluster, so 2 MiB of virtual disk. Every other fixture here is
/// 16 KiB, which means `l1_index` is 0 in every test, and forcing it to
/// 0 in the reader failed **no** tests at all: the second level of the
/// two-level addressing was never exercised.
///
/// This one is 3 MiB, so virtual offsets past 2 MiB land in the second
/// L1 entry and a wrong `l1_index` reads the wrong L2 table.
///
/// Layout:
///
/// ```text
///   cluster 0     header
///   cluster 1     L1 table (2 entries)
///   cluster 2     L2 table for L1 entry 0
///   cluster 3     L2 table for L1 entry 1
///   cluster 4     data for virt cluster 0        (0xA1)
///   cluster 5     data for virt cluster 512      (0xB2, first of L1 #1)
///   cluster 6     refcount table
///   cluster 7     refcount block
/// ```
pub fn build_two_l1_entry_image(path: &Path) {
    const VIRT: u64 = 3 * 1024 * 1024;
    const L1: u64 = CLUSTER_SIZE;
    const L2_A: u64 = CLUSTER_SIZE * 2;
    const L2_B: u64 = CLUSTER_SIZE * 3;
    const DATA_A: u64 = CLUSTER_SIZE * 4;
    const DATA_B: u64 = CLUSTER_SIZE * 5;
    const RT: u64 = CLUSTER_SIZE * 6;
    const RB: u64 = CLUSTER_SIZE * 7;
    const CLUSTERS: u64 = 32;

    let mut f = File::create(path).unwrap();
    f.set_len(CLUSTER_SIZE * CLUSTERS).unwrap();

    let mut hdr = [0u8; 4096];
    hdr[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
    hdr[4..8].copy_from_slice(&3u32.to_be_bytes());
    hdr[20..24].copy_from_slice(&12u32.to_be_bytes());
    hdr[24..32].copy_from_slice(&VIRT.to_be_bytes());
    hdr[36..40].copy_from_slice(&2u32.to_be_bytes()); // l1_size = 2
    hdr[40..48].copy_from_slice(&L1.to_be_bytes());
    hdr[48..56].copy_from_slice(&RT.to_be_bytes());
    hdr[56..60].copy_from_slice(&1u32.to_be_bytes());
    hdr[96..100].copy_from_slice(&4u32.to_be_bytes());
    hdr[100..104].copy_from_slice(&104u32.to_be_bytes());
    f.write_all_at(&hdr, 0).unwrap();

    let mut l1 = [0u8; 4096];
    l1[0..8].copy_from_slice(&((L2_A & HOST_OFFSET_MASK) | COPIED).to_be_bytes());
    l1[8..16].copy_from_slice(&((L2_B & HOST_OFFSET_MASK) | COPIED).to_be_bytes());
    f.write_all_at(&l1, L1).unwrap();

    // L2 for L1 entry 0: virt cluster 0 → DATA_A.
    let mut l2a = [0u8; 4096];
    l2a[0..8].copy_from_slice(&((DATA_A & HOST_OFFSET_MASK) | COPIED).to_be_bytes());
    f.write_all_at(&l2a, L2_A).unwrap();

    // L2 for L1 entry 1: virt cluster 512 (its entry 0) → DATA_B.
    let mut l2b = [0u8; 4096];
    l2b[0..8].copy_from_slice(&((DATA_B & HOST_OFFSET_MASK) | COPIED).to_be_bytes());
    f.write_all_at(&l2b, L2_B).unwrap();

    f.write_all_at(&[0xA1u8; 4096], DATA_A).unwrap();
    f.write_all_at(&[0xB2u8; 4096], DATA_B).unwrap();

    let mut rt = [0u8; 4096];
    rt[0..8].copy_from_slice(&RB.to_be_bytes());
    f.write_all_at(&rt, RT).unwrap();

    let mut rb = [0u8; 4096];
    for cluster_idx in 0..8u16 {
        let off = (cluster_idx as usize) * 2;
        rb[off..off + 2].copy_from_slice(&1u16.to_be_bytes());
    }
    f.write_all_at(&rb, RB).unwrap();
}
