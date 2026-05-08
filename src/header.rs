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
//! ```
//!
//! All multi-byte fields are big-endian.

use crate::error::{Error, Result};

pub const QCOW2_MAGIC: u32 = 0x5146_49fb; // "QFI\xfb"

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
}

impl Header {
    /// Parse from the first 104 bytes of the file. `bytes` may be longer; only
    /// the prefix is read.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 72 {
            return Err(Error::Corrupt("header shorter than 72 bytes"));
        }

        let magic = read_u32(bytes, 0);
        if magic != QCOW2_MAGIC {
            return Err(Error::NotQcow2);
        }

        let version = read_u32(bytes, 4);
        if version != 2 && version != 3 {
            return Err(Error::UnsupportedVersion(version));
        }

        let backing_file_offset = read_u64(bytes, 8);
        let backing_file_size = read_u32(bytes, 16);
        let cluster_bits = read_u32(bytes, 20);
        if !(9..=21).contains(&cluster_bits) {
            return Err(Error::InvalidClusterBits(cluster_bits));
        }
        let cluster_size = 1u64 << cluster_bits;

        let virtual_size = read_u64(bytes, 24);
        let crypt_method = read_u32(bytes, 32);
        let l1_size = read_u32(bytes, 36);
        let l1_table_offset = read_u64(bytes, 40);
        let refcount_table_offset = read_u64(bytes, 48);
        let refcount_table_clusters = read_u32(bytes, 56);
        let nb_snapshots = read_u32(bytes, 60);
        let snapshots_offset = read_u64(bytes, 64);

        let (
            incompatible_features,
            compatible_features,
            autoclear_features,
            refcount_order,
            header_length,
        ) = if version >= 3 {
            if bytes.len() < 104 {
                return Err(Error::Corrupt("v3 header shorter than 104 bytes"));
            }
            (
                read_u64(bytes, 72),
                read_u64(bytes, 80),
                read_u64(bytes, 88),
                read_u32(bytes, 96),
                read_u32(bytes, 100),
            )
        } else {
            // v2 implicit defaults per spec.
            (0, 0, 0, 4, 72)
        };

        // Sanity: l1 must be large enough to address the whole virtual disk.
        let l2_entries_per_cluster = cluster_size / 8;
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
        })
    }

    /// Number of u64 entries in one L2 table.
    pub fn l2_entries(&self) -> u64 {
        self.cluster_size / 8
    }

    /// Reject feature combinations the reader does not yet handle. Currently
    /// supported: uncompressed clusters, zlib-compressed clusters, backing
    /// files. Refused: encryption, external data file, non-zlib compression,
    /// extended L2 entries.
    pub fn check_supported(&self) -> Result<()> {
        if self.crypt_method != 0 {
            return Err(Error::Unsupported("encryption (AES or LUKS)"));
        }
        if self.version >= 3 {
            let unknown = self.incompatible_features & !(incompat::DIRTY | incompat::CORRUPT);
            if unknown != 0 {
                if self.incompatible_features & incompat::DATA_FILE != 0 {
                    return Err(Error::Unsupported("external data file"));
                }
                if self.incompatible_features & incompat::COMPRESSION_TYPE != 0 {
                    return Err(Error::Unsupported("non-zlib compression"));
                }
                if self.incompatible_features & incompat::EXTENDED_L2 != 0 {
                    return Err(Error::Unsupported("extended L2 entries"));
                }
                return Err(Error::Unsupported("unknown incompatible_features bit"));
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
}
