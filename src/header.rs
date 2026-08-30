//! On-disk header parser. Layout per the QCOW2 spec:
//!
//! ```text
//!   0   u32  magic = "QFI\xfb" (0x51 0x46 0x49 0xfb)
//!   4   u32  version (2 or 3)
//!   8   u64  backing_file_offset
//!  16   u32  backing_file_size
//!  20   u32  cluster_bits        (cluster_size = 1 << cluster_bits)
//!  24   u64  size                (virtual disk size in bytes)
//!  32   u32  crypt_method        (0 = none, 1 = AES, 2 = LUKS)
//!  36   u32  l1_size             (number of u64 entries in L1)
//!  40   u64  l1_table_offset
//!  48   u64  refcount_table_offset
//!  56   u32  refcount_table_clusters
//!  60   u32  nb_snapshots
//!  64   u64  snapshots_offset
//!  --- v3 only ---
//!  72   u64  incompatible_features
//!  80   u64  compatible_features
//!  88   u64  autoclear_features
//!  96   u32  refcount_order      (refcount_bits = 1 << refcount_order)
//! 100   u32  header_length
//!  --- v3 additional fields (only present when header_length > 104) ---
//! 104   u8   compression_type    (0 = zlib, 1 = zstd)
//! 105   7B   padding
//! ```
//!
//! All multi-byte fields are big-endian.

use crate::error::{Error, Result};

pub const QCOW2_MAGIC: u32 = 0x5146_49fb; // "QFI\xfb"

/// Byte offsets of the header's fields, per the layout above.
///
/// The parser reads through these; the *fixtures* deliberately do not,
/// and write literal offsets instead. That is what lets a wrong
/// constant here be caught rather than agreed with: moving
/// `CLUSTER_BITS` by one byte fails 52 tests, `L1_TABLE_OFFSET` 35.
/// Rewriting the fixtures against this table would take both to zero.
/// [`tests::offsets_match_the_published_specification`] is the same
/// intent written down once, so it survives a later tidy-up.
pub mod offsets {
    pub const MAGIC: usize = 0;
    pub const VERSION: usize = 4;
    pub const BACKING_FILE_OFFSET: usize = 8;
    pub const BACKING_FILE_SIZE: usize = 16;
    pub const CLUSTER_BITS: usize = 20;
    pub const SIZE: usize = 24;
    pub const CRYPT_METHOD: usize = 32;
    pub const L1_SIZE: usize = 36;
    pub const L1_TABLE_OFFSET: usize = 40;
    pub const REFCOUNT_TABLE_OFFSET: usize = 48;
    pub const REFCOUNT_TABLE_CLUSTERS: usize = 56;
    pub const NB_SNAPSHOTS: usize = 60;
    pub const SNAPSHOTS_OFFSET: usize = 64;

    // v3 only.
    pub const INCOMPATIBLE_FEATURES: usize = 72;
    pub const COMPATIBLE_FEATURES: usize = 80;
    pub const AUTOCLEAR_FEATURES: usize = 88;
    pub const REFCOUNT_ORDER: usize = 96;
    pub const HEADER_LENGTH: usize = 100;

    /// Where the v3 header ends if it carries no additional fields.
    /// A `header_length` above this means `COMPRESSION_TYPE` is present.
    pub const V3_BASE_HEADER_LENGTH: u32 = 104;
    /// First byte of the v3 additional-fields region.
    pub const COMPRESSION_TYPE: usize = 104;

    /// v2 has no feature words, no `refcount_order` and no
    /// `header_length` on disk. The specification fixes their values,
    /// and these are them — not defaults this crate chose.
    pub mod v2_implicit {
        pub const REFCOUNT_ORDER: u32 = 4;
        pub const HEADER_LENGTH: u32 = 72;
    }
}

/// Bits set in `incompatible_features` we know how to refuse cleanly.
pub mod incompat {
    pub const DIRTY: u64 = 1 << 0;
    pub const CORRUPT: u64 = 1 << 1;
    pub const DATA_FILE: u64 = 1 << 2; // external data file
    pub const COMPRESSION_TYPE: u64 = 1 << 3; // non-zlib compression declared
    pub const EXTENDED_L2: u64 = 1 << 4;
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: u32,
    pub cluster_bits: u32,
    pub cluster_size: u64,
    pub virtual_size: u64,
    pub crypt_method: u32,

    pub l1_size: u32,
    pub l1_table_offset: u64,

    pub refcount_table_offset: u64,
    pub refcount_table_clusters: u32,

    pub nb_snapshots: u32,
    pub snapshots_offset: u64,

    pub backing_file_offset: u64,
    pub backing_file_size: u32,

    /// v3 only — zero for v2 images.
    pub incompatible_features: u64,
    pub compatible_features: u64,
    pub autoclear_features: u64,
    pub refcount_order: u32,
    pub header_length: u32,

    /// v3 additional field at byte 104. Present only when
    /// `header_length > 104`; defaults to 0 (zlib) otherwise.
    /// Spec values: 0 = zlib (deflate), 1 = zstd.
    pub compression_type: u8,
}

impl Header {
    /// Parse from the first 104 bytes of the file. `bytes` may be longer; only
    /// the prefix is read.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 72 {
            return Err(Error::Corrupt("header shorter than 72 bytes"));
        }

        let magic = read_u32(bytes, offsets::MAGIC);
        if magic != QCOW2_MAGIC {
            return Err(Error::NotQcow2);
        }

        let version = read_u32(bytes, offsets::VERSION);
        if version != 2 && version != 3 {
            return Err(Error::UnsupportedVersion(version));
        }

        let backing_file_offset = read_u64(bytes, offsets::BACKING_FILE_OFFSET);
        let backing_file_size = read_u32(bytes, offsets::BACKING_FILE_SIZE);
        let cluster_bits = read_u32(bytes, offsets::CLUSTER_BITS);
        if !(9..=21).contains(&cluster_bits) {
            return Err(Error::InvalidClusterBits(cluster_bits));
        }
        let cluster_size = 1u64 << cluster_bits;

        let virtual_size = read_u64(bytes, offsets::SIZE);
        let crypt_method = read_u32(bytes, offsets::CRYPT_METHOD);
        let l1_size = read_u32(bytes, offsets::L1_SIZE);
        let l1_table_offset = read_u64(bytes, offsets::L1_TABLE_OFFSET);
        let refcount_table_offset = read_u64(bytes, offsets::REFCOUNT_TABLE_OFFSET);
        let refcount_table_clusters = read_u32(bytes, offsets::REFCOUNT_TABLE_CLUSTERS);
        let nb_snapshots = read_u32(bytes, offsets::NB_SNAPSHOTS);
        let snapshots_offset = read_u64(bytes, offsets::SNAPSHOTS_OFFSET);

        let (
            incompatible_features,
            compatible_features,
            autoclear_features,
            refcount_order,
            header_length,
        ) = if version >= 3 {
            if bytes.len() < offsets::V3_BASE_HEADER_LENGTH as usize {
                return Err(Error::Corrupt("v3 header shorter than 104 bytes"));
            }
            (
                read_u64(bytes, offsets::INCOMPATIBLE_FEATURES),
                read_u64(bytes, offsets::COMPATIBLE_FEATURES),
                read_u64(bytes, offsets::AUTOCLEAR_FEATURES),
                read_u32(bytes, offsets::REFCOUNT_ORDER),
                read_u32(bytes, offsets::HEADER_LENGTH),
            )
        } else {
            // v2 implicit defaults per spec.
            (
                0,
                0,
                0,
                offsets::v2_implicit::REFCOUNT_ORDER,
                offsets::v2_implicit::HEADER_LENGTH,
            )
        };

        // v3 additional-fields region starts at byte 104. The first byte is
        // `compression_type` (0 = zlib, 1 = zstd). Older v3 images set
        // header_length to 104 and omit this byte; treat as zlib.
        let compression_type = if version >= 3
            && header_length > offsets::V3_BASE_HEADER_LENGTH
            && bytes.len() > offsets::COMPRESSION_TYPE
        {
            bytes[offsets::COMPRESSION_TYPE]
        } else {
            0
        };

        // Sanity: l1 must be large enough to address the whole virtual disk.
        let l2_entries_per_cluster = cluster_size / crate::reader::TABLE_ENTRY_BYTES;
        let bytes_per_l1_entry = cluster_size * l2_entries_per_cluster;
        let needed_l1 = virtual_size.div_ceil(bytes_per_l1_entry);
        if (l1_size as u64) < needed_l1 {
            return Err(Error::Corrupt("l1_size too small for virtual_size"));
        }

        Ok(Header {
            version,
            cluster_bits,
            cluster_size,
            virtual_size,
            crypt_method,
            l1_size,
            l1_table_offset,
            refcount_table_offset,
            refcount_table_clusters,
            nb_snapshots,
            snapshots_offset,
            backing_file_offset,
            backing_file_size,
            incompatible_features,
            compatible_features,
            autoclear_features,
            refcount_order,
            header_length,
            compression_type,
        })
    }

    /// Number of u64 entries in one L2 table.
    pub fn l2_entries(&self) -> u64 {
        self.cluster_size / 8
    }

    /// Reject feature combinations the reader does not yet handle. Currently
    /// supported: uncompressed clusters, zlib- and zstd-compressed clusters,
    /// backing files. Refused: encryption, external data file, extended L2
    /// entries, unknown compression types.
    pub fn check_supported(&self) -> Result<()> {
        if self.crypt_method != 0 {
            return Err(Error::Unsupported("encryption (AES or LUKS)"));
        }
        if self.version >= 3 {
            // COMPRESSION_TYPE in incompatible_features means "compression
            // is something other than the default (zlib)". We honour it as
            // long as compression_type itself is a value we implement.
            let known = incompat::DIRTY | incompat::CORRUPT | incompat::COMPRESSION_TYPE;
            let unknown = self.incompatible_features & !known;
            if unknown != 0 {
                if self.incompatible_features & incompat::DATA_FILE != 0 {
                    return Err(Error::Unsupported("external data file"));
                }
                if self.incompatible_features & incompat::EXTENDED_L2 != 0 {
                    return Err(Error::Unsupported("extended L2 entries"));
                }
                return Err(Error::Unsupported("unknown incompatible_features bit"));
            }
            // 0 = zlib, 1 = zstd. Anything else is something we don't know
            // how to decode.
            if self.compression_type > 1 {
                return Err(Error::Unsupported("unknown compression_type"));
            }
        }
        Ok(())
    }
}

fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn read_u64(b: &[u8], off: usize) -> u64 {
    u64::from_be_bytes([
        b[off],
        b[off + 1],
        b[off + 2],
        b[off + 3],
        b[off + 4],
        b[off + 5],
        b[off + 6],
        b[off + 7],
    ])
}

#[cfg(test)]
mod tests {
    /// The offset table, checked against the specification's own table.
    ///
    /// A deliberate second copy: the literals here come from the QCOW2
    /// specification, reproduced in this module's doc comment. Every
    /// other reader in the crate now goes through the constants, so
    /// this is the only place that can disagree with them.
    ///
    /// If this test and the constants diverge, re-read the spec before
    /// touching either — this is the half with independent provenance.
    #[test]
    fn offsets_match_the_published_specification() {
        use super::offsets as at;
        assert_eq!(at::MAGIC, 0);
        assert_eq!(at::VERSION, 4);
        assert_eq!(at::BACKING_FILE_OFFSET, 8);
        assert_eq!(at::BACKING_FILE_SIZE, 16);
        assert_eq!(at::CLUSTER_BITS, 20);
        assert_eq!(at::SIZE, 24);
        assert_eq!(at::CRYPT_METHOD, 32);
        assert_eq!(at::L1_SIZE, 36);
        assert_eq!(at::L1_TABLE_OFFSET, 40);
        assert_eq!(at::REFCOUNT_TABLE_OFFSET, 48);
        assert_eq!(at::REFCOUNT_TABLE_CLUSTERS, 56);
        assert_eq!(at::NB_SNAPSHOTS, 60);
        assert_eq!(at::SNAPSHOTS_OFFSET, 64);
        assert_eq!(at::INCOMPATIBLE_FEATURES, 72);
        assert_eq!(at::COMPATIBLE_FEATURES, 80);
        assert_eq!(at::AUTOCLEAR_FEATURES, 88);
        assert_eq!(at::REFCOUNT_ORDER, 96);
        assert_eq!(at::HEADER_LENGTH, 100);
        assert_eq!(at::V3_BASE_HEADER_LENGTH, 104);
        assert_eq!(at::COMPRESSION_TYPE, 104);
        assert_eq!(at::v2_implicit::REFCOUNT_ORDER, 4);
        assert_eq!(at::v2_implicit::HEADER_LENGTH, 72);
    }

    /// No header field overlaps the next.
    ///
    /// The table above pins each offset on its own; this checks they
    /// still describe one layout. A field widened without its
    /// neighbour moving satisfies every assertion above and fails here.
    #[test]
    fn no_header_field_overlaps_its_neighbour() {
        use super::offsets as at;
        let fields = [
            (at::MAGIC, 4),
            (at::VERSION, 4),
            (at::BACKING_FILE_OFFSET, 8),
            (at::BACKING_FILE_SIZE, 4),
            (at::CLUSTER_BITS, 4),
            (at::SIZE, 8),
            (at::CRYPT_METHOD, 4),
            (at::L1_SIZE, 4),
            (at::L1_TABLE_OFFSET, 8),
            (at::REFCOUNT_TABLE_OFFSET, 8),
            (at::REFCOUNT_TABLE_CLUSTERS, 4),
            (at::NB_SNAPSHOTS, 4),
            (at::SNAPSHOTS_OFFSET, 8),
            (at::INCOMPATIBLE_FEATURES, 8),
            (at::COMPATIBLE_FEATURES, 8),
            (at::AUTOCLEAR_FEATURES, 8),
            (at::REFCOUNT_ORDER, 4),
            (at::HEADER_LENGTH, 4),
            (at::COMPRESSION_TYPE, 1),
        ];
        let mut reached = 0usize;
        for (start, width) in fields {
            assert!(
                start >= reached,
                "field at {start} overlaps the one ending at {reached}"
            );
            reached = start + width;
        }
        // The v3 base header ends exactly where the additional-fields
        // region begins.
        assert_eq!(at::V3_BASE_HEADER_LENGTH as usize, at::COMPRESSION_TYPE);
    }

    use super::*;

    #[test]
    fn rejects_bad_magic() {
        let bytes = [0u8; 104];
        assert!(matches!(Header::parse(&bytes), Err(Error::NotQcow2)));
    }

    #[test]
    fn rejects_bad_version() {
        let mut bytes = [0u8; 104];
        bytes[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
        bytes[4..8].copy_from_slice(&7u32.to_be_bytes());
        assert!(matches!(
            Header::parse(&bytes),
            Err(Error::UnsupportedVersion(7))
        ));
    }

    /// Build a parseable v3 header (112 bytes so the compression_type
    /// byte at offset 104 is read). cluster_bits = 16 (64 KiB clusters),
    /// virtual_size = 1 MiB, l1_size = 1 (the minimum that addresses it).
    fn valid_v3_header() -> Vec<u8> {
        let mut b = vec![0u8; 112];
        b[0..4].copy_from_slice(&QCOW2_MAGIC.to_be_bytes());
        b[4..8].copy_from_slice(&3u32.to_be_bytes()); // version 3
        b[20..24].copy_from_slice(&16u32.to_be_bytes()); // cluster_bits
        b[24..32].copy_from_slice(&(1u64 << 20).to_be_bytes()); // virtual_size = 1 MiB
        b[36..40].copy_from_slice(&1u32.to_be_bytes()); // l1_size
        b[96..100].copy_from_slice(&4u32.to_be_bytes()); // refcount_order
        b[100..104].copy_from_slice(&112u32.to_be_bytes()); // header_length
        b
    }

    fn set_u32(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_be_bytes());
    }
    fn set_u64(b: &mut [u8], off: usize, v: u64) {
        b[off..off + 8].copy_from_slice(&v.to_be_bytes());
    }

    #[test]
    fn parses_valid_v3_header_fields() {
        let h = Header::parse(&valid_v3_header()).unwrap();
        assert_eq!(h.version, 3);
        assert_eq!(h.cluster_bits, 16);
        assert_eq!(h.cluster_size, 1 << 16);
        assert_eq!(h.virtual_size, 1 << 20);
        assert_eq!(h.refcount_order, 4);
        assert_eq!(h.l2_entries(), (1 << 16) / 8);
        h.check_supported().unwrap();
    }

    #[test]
    fn v2_header_uses_implicit_defaults() {
        let mut b = valid_v3_header();
        set_u32(&mut b, 4, 2); // version 2
        let h = Header::parse(&b).unwrap();
        // v2 has no feature fields; spec defaults apply.
        assert_eq!(h.incompatible_features, 0);
        assert_eq!(h.refcount_order, 4);
        assert_eq!(h.header_length, 72);
        assert_eq!(h.compression_type, 0);
        h.check_supported().unwrap();
    }

    #[test]
    fn rejects_cluster_bits_outside_9_to_21() {
        for bad in [0u32, 8, 22, 31] {
            let mut b = valid_v3_header();
            set_u32(&mut b, 20, bad);
            match Header::parse(&b) {
                Err(Error::InvalidClusterBits(v)) => assert_eq!(v, bad),
                other => panic!("cluster_bits {bad}: expected InvalidClusterBits, got {other:?}"),
            }
        }
        // Boundaries 9 and 21 are accepted. Use a small virtual_size so
        // l1_size = 1 addresses it even at cluster_bits = 9 (where one L1
        // entry only maps 32 KiB).
        for ok in [9u32, 21] {
            let mut b = valid_v3_header();
            set_u32(&mut b, 20, ok);
            set_u64(&mut b, 24, 16 * 1024); // 16 KiB virtual disk
            Header::parse(&b).unwrap_or_else(|e| panic!("cluster_bits {ok} should parse: {e:?}"));
        }
    }

    #[test]
    fn rejects_l1_size_too_small_for_virtual_size() {
        let mut b = valid_v3_header();
        // Keep l1_size = 1 but blow up virtual_size so one L1 entry can't
        // cover it. At cluster_bits=16 one L1 entry maps 512 MiB, so ask
        // for 2 GiB.
        set_u64(&mut b, 24, 2u64 << 30);
        match Header::parse(&b) {
            Err(Error::Corrupt(_)) => {}
            other => panic!("expected Corrupt(l1_size too small), got {other:?}"),
        }
    }

    #[test]
    fn rejects_v3_header_shorter_than_104_bytes() {
        let mut b = valid_v3_header();
        b.truncate(96);
        match Header::parse(&b) {
            Err(Error::Corrupt(_)) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn compression_type_byte_only_read_when_header_is_long_enough() {
        // header_length = 104 → the compression_type byte is omitted and
        // defaults to zlib (0) even if byte 104 happens to be non-zero.
        let mut b = valid_v3_header();
        set_u32(&mut b, 100, 104); // header_length = 104
        b[104] = 1; // would be zstd if it were read
        let h = Header::parse(&b).unwrap();
        assert_eq!(
            h.compression_type, 0,
            "byte past header_length must be ignored"
        );
    }

    #[test]
    fn check_supported_rejects_encryption() {
        let mut b = valid_v3_header();
        set_u32(&mut b, 32, 1); // crypt_method = AES
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert!(m.contains("encryption")),
            other => panic!("expected Unsupported(encryption), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_rejects_external_data_file() {
        let mut b = valid_v3_header();
        set_u64(&mut b, 72, incompat::DATA_FILE);
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert_eq!(m, "external data file"),
            other => panic!("expected Unsupported(external data file), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_rejects_extended_l2() {
        let mut b = valid_v3_header();
        set_u64(&mut b, 72, incompat::EXTENDED_L2);
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert_eq!(m, "extended L2 entries"),
            other => panic!("expected Unsupported(extended L2 entries), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_rejects_unknown_incompatible_bit() {
        let mut b = valid_v3_header();
        set_u64(&mut b, 72, 1 << 20); // a bit we don't recognise
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert_eq!(m, "unknown incompatible_features bit"),
            other => panic!("expected Unsupported(unknown bit), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_rejects_unknown_compression_type() {
        let mut b = valid_v3_header();
        // Declare COMPRESSION_TYPE and an out-of-range compression byte.
        set_u64(&mut b, 72, incompat::COMPRESSION_TYPE);
        b[104] = 2; // neither zlib(0) nor zstd(1)
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert_eq!(m, "unknown compression_type"),
            other => panic!("expected Unsupported(unknown compression_type), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_accepts_zstd_with_compression_type_bit() {
        let mut b = valid_v3_header();
        set_u64(&mut b, 72, incompat::COMPRESSION_TYPE);
        b[104] = 1; // zstd
        let h = Header::parse(&b).unwrap();
        assert_eq!(h.compression_type, 1);
        h.check_supported().expect("zstd is supported");
    }

    #[test]
    fn check_supported_rejects_bad_compression_type_even_without_the_flag() {
        // check_supported gates `compression_type > 1` unconditionally for
        // v3, independent of whether the COMPRESSION_TYPE incompatible bit
        // is declared. Exercise the flag-absent path: an out-of-range byte
        // at offset 104 must still be refused even though
        // incompatible_features is zero.
        let mut b = valid_v3_header();
        // incompatible_features left at 0 (no COMPRESSION_TYPE flag).
        b[104] = 2; // neither zlib(0) nor zstd(1)
        let h = Header::parse(&b).unwrap();
        match h.check_supported() {
            Err(Error::Unsupported(m)) => assert_eq!(m, "unknown compression_type"),
            other => panic!("expected Unsupported(unknown compression_type), got {other:?}"),
        }
    }

    #[test]
    fn check_supported_tolerates_dirty_and_corrupt_advisory_bits() {
        // DIRTY and CORRUPT are recognised (and tolerated) incompatible
        // bits — they must not trip the unknown-bit rejection.
        let mut b = valid_v3_header();
        set_u64(&mut b, 72, incompat::DIRTY | incompat::CORRUPT);
        let h = Header::parse(&b).unwrap();
        h.check_supported().expect("dirty/corrupt are tolerated");
    }
}
