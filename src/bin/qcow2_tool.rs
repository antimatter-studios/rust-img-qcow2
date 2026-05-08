//! Command-line introspection tool for QCOW2 images.
//!
//! Usage:
//!   qcow2_tool info <file>
//!   qcow2_tool read <file> <offset> <len>      (hex dump to stdout)
//!   qcow2_tool dump <file> <offset> <len>      (raw bytes to stdout)
//!
//! Numbers accept hex (`0x...`) or decimal.

use qcow2::Qcow2Reader;
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("info") => cmd_info(&args[2..]),
        Some("read") => cmd_read(&args[2..], false),
        Some("dump") => cmd_read(&args[2..], true),
        Some("--help") | Some("-h") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => Err(format!("unknown subcommand: {other}")),
    };
    match rc {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("qcow2_tool: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "qcow2_tool — QCOW2 inspector

Usage:
  qcow2_tool info <file>
  qcow2_tool read <file> <offset> <len>     hex dump to stdout
  qcow2_tool dump <file> <offset> <len>     raw bytes to stdout

Numbers accept hex (0x...) or decimal."
    );
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    if args.len() != 1 {
        return Err("info: expected <file>".into());
    }
    let r = Qcow2Reader::open(&args[0]).map_err(|e| e.to_string())?;
    let h = r.header();
    println!("version            : {}", h.version);
    println!("cluster_bits       : {}", h.cluster_bits);
    println!(
        "cluster_size       : {} ({:#x})",
        h.cluster_size, h.cluster_size
    );
    println!(
        "virtual_size       : {} bytes ({:.2} MiB)",
        h.virtual_size,
        (h.virtual_size as f64) / (1024.0 * 1024.0)
    );
    println!("crypt_method       : {}", h.crypt_method);
    println!("l1_size            : {}", h.l1_size);
    println!("l1_table_offset    : {:#x}", h.l1_table_offset);
    println!("refcount_table_off : {:#x}", h.refcount_table_offset);
    println!("refcount_table_clu : {}", h.refcount_table_clusters);
    println!("nb_snapshots       : {}", h.nb_snapshots);
    println!("snapshots_offset   : {:#x}", h.snapshots_offset);
    if h.version >= 3 {
        println!("incompat_features  : {:#x}", h.incompatible_features);
        println!("compat_features    : {:#x}", h.compatible_features);
        println!("autoclear_features : {:#x}", h.autoclear_features);
        println!("refcount_order     : {}", h.refcount_order);
        println!("header_length      : {}", h.header_length);
    }
    Ok(())
}

fn cmd_read(args: &[String], raw: bool) -> Result<(), String> {
    if args.len() != 3 {
        return Err("read/dump: expected <file> <offset> <len>".into());
    }
    let r = Qcow2Reader::open(&args[0]).map_err(|e| e.to_string())?;
    let offset = parse_u64(&args[1])?;
    let len = parse_u64(&args[2])?;
    if len > 64 * 1024 * 1024 {
        return Err("len too large (cap 64 MiB)".into());
    }
    let mut buf = vec![0u8; len as usize];
    r.read_at(offset, &mut buf).map_err(|e| e.to_string())?;

    if raw {
        std::io::stdout()
            .write_all(&buf)
            .map_err(|e| e.to_string())?;
    } else {
        hex_dump(offset, &buf);
    }
    Ok(())
}

fn parse_u64(s: &str) -> Result<u64, String> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).map_err(|e| format!("invalid hex {s:?}: {e}"))
    } else {
        s.parse::<u64>()
            .map_err(|e| format!("invalid decimal {s:?}: {e}"))
    }
}

fn hex_dump(start: u64, bytes: &[u8]) {
    for (i, line) in bytes.chunks(16).enumerate() {
        let off = start + (i as u64) * 16;
        print!("{off:08x}  ");
        for (j, b) in line.iter().enumerate() {
            print!("{b:02x}");
            if j == 7 {
                print!(" ");
            }
            print!(" ");
        }
        for _ in line.len()..16 {
            print!("   ");
        }
        print!(" |");
        for b in line {
            let c = *b;
            let printable = (0x20..0x7f).contains(&c);
            print!("{}", if printable { c as char } else { '.' });
        }
        println!("|");
    }
}
