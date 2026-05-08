//! End-to-end demo: open a qcow2, probe its partition table, sniff the FS
//! signature on each partition. Run with:
//!
//!     cargo run --example inspect -- /path/to/disk.qcow2

use partitions::{probe, sniff, FsKind};
use qcow2::Qcow2Reader;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: inspect <file.qcow2>");
            std::process::exit(2);
        }
    };

    let reader = match Qcow2Reader::open(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("open {path}: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "qcow2: version {}, virtual size {} bytes, cluster {} bytes",
        reader.version(),
        reader.virtual_size(),
        reader.cluster_size()
    );

    let (table, parts) = match probe(&reader) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("probe: {e}");
            std::process::exit(1);
        }
    };
    println!("table: {table:?}, {} partition(s)", parts.len());

    for (i, p) in parts.iter().enumerate() {
        let kind = sniff(&reader, p).unwrap_or(FsKind::Unknown);
        let label = p.label.as_deref().unwrap_or("");
        println!(
            "  [{i}] start=0x{:x} length={} fs={:?} label={:?}",
            p.start, p.length, kind, label
        );
    }
}
