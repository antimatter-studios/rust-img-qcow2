//! `img.qcow2 <image> <verb>`: an errand inside a QCOW2 disk image, without
//! a hypervisor.
//!
//! The verbs are the shared set for disk images: `info`/`get`, `read`,
//! `write`, `create`, `resize`, `set`. Metadata is JSON (or `--text`); the
//! guest's bytes are raw. `read` with no range streams the whole virtual
//! disk, backing chain resolved and compressed clusters inflated, so
//! converting to a raw image is reading it. A verb the library cannot do
//! yet still exists and answers `not implemented` with exit status 3, so a
//! script moved between formats fails loudly instead of meaning something
//! else.

use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use clap::{value_parser, Arg, ArgMatches, Command as Cmd};

use crate::common::{CliError, Json, Outcome, Tool};
use qcow2::Qcow2Reader;

pub const TOOL: Tool = Tool {
    name: "img.qcow2",
    verb: "img",
    section: 1,
    usage_exit: crate::common::output::EXIT_USAGE,
    about: "Report, read and write a QCOW2 disk image without a hypervisor",
    command,
    run,
};

/// The canonical keys every `img.<fmt>` answers, in the shared order.
/// The format's own fields are nested under `qcow2`.
pub const KEYS: &[&str] = &[
    "format",
    "virtual_size",
    "block_size",
    "backing",
    "dirty",
    "qcow2",
];

/// How much of the guest is moved at a time.
const CHUNK: usize = 1 << 20;

fn command() -> Cmd {
    Cmd::new("img.qcow2")
        .about("Report, read and write a QCOW2 disk image without a hypervisor")
        .long_about(
            "Work inside a QCOW2 disk image directly: versions 2 and 3, backing chains, \
             zlib and zstd compressed clusters, internal snapshots; no hypervisor and no \
             conversion tool.\n\n\
             Metadata is JSON on stdout (--text for people); `read` writes the guest's raw \
             bytes, the whole virtual disk when no range is given, so converting to a raw \
             image is reading it. A failure is {\"error\": \"...\", \"code\": N} on stderr, \
             N being the exit status: 1 failed, 2 wrong command line, 3 not implemented.",
        )
        .arg(
            Arg::new("image")
                .value_name("IMAGE")
                .help("The QCOW2 image file")
                .value_parser(value_parser!(OsString))
                .required(true),
        )
        .args(crate::common::format_args().map(|a| a.global(true)))
        .subcommand_required(true)
        .subcommand(key_command(
            "info",
            "Report the image's properties, or one of them",
        ))
        .subcommand(key_command(
            "get",
            "The same as info: every property, or one of them",
        ))
        .subcommand(
            Cmd::new("read")
                .about("Write the guest's bytes to stdout, or to a file with -o: the whole disk, or a range")
                .arg(byte_count("offset", "Where to start, in the guest (default 0)"))
                .arg(byte_count(
                    "length",
                    "How many bytes (default: to the end of the virtual disk)",
                ))
                .arg(
                    Arg::new("output")
                        .short('o')
                        .long("output")
                        .value_name("FILE")
                        .value_parser(value_parser!(OsString))
                        .help("Write here instead of stdout (zero runs are left sparse)"),
                )
                .after_help(
                    "Examples:\n  img.qcow2 disk.qcow2 read -o disk.raw\n  \
                     img.qcow2 disk.qcow2 read --offset 0 --length 512 | xxd\n  \
                     img.qcow2 disk.qcow2 read --offset 1M --length 64K > chunk.bin\n\n\
                     A backing chain is followed and compressed clusters are inflated: the \
                     bytes are the ones the guest sees.",
                ),
        )
        .subcommand(
            Cmd::new("write")
                .about("Write the bytes on stdin into the guest at an offset")
                .arg(byte_count("offset", "Where to write, in the guest").required(true))
                .after_help(
                    "Examples:\n  img.qcow2 disk.qcow2 write --offset 0 < mbr.bin\n  \
                     img.qcow2 src.qcow2 read | img.qcow2 dst.qcow2 write --offset 0\n  \
                     printf 'hello' | img.qcow2 disk.qcow2 write --offset 1M\n\n\
                     Input that would run past the end of the virtual disk is refused before \
                     anything is written. Clusters are allocated, copied from the backing file \
                     or out of a snapshot, and inflated from compression as the write needs; \
                     writes land in IMAGE only, never in its backing file. An image flagged \
                     dirty or corrupt answers `not implemented` (exit 3): the library does \
                     not repair refcounts.",
                ),
        )
        .subcommand(
            Cmd::new("create")
                .about("Create a new, empty image (not implemented)")
                .arg(Arg::new("size").value_name("SIZE").required(true))
                .after_help(
                    "Examples:\n  img.qcow2 new.qcow2 create 64M\n\n\
                     Answers `not implemented` (exit 3): the library has no image creator.",
                ),
        )
        .subcommand(
            Cmd::new("set")
                .about("Change a property (not implemented)")
                .arg(Arg::new("key").value_name("KEY").required(true))
                .arg(Arg::new("value").value_name("VALUE").required(true))
                .after_help(
                    "Examples:\n  img.qcow2 disk.qcow2 set backing base.qcow2\n\n\
                     Answers `not implemented` (exit 3): the library changes no header field.",
                ),
        )
        .subcommand(
            Cmd::new("resize")
                .about("Grow or shrink the virtual disk (not implemented)")
                .arg(Arg::new("size").value_name("SIZE").required(true))
                .after_help(
                    "Examples:\n  img.qcow2 disk.qcow2 resize 20G\n\n\
                     Answers `not implemented` (exit 3): the library has no resize.",
                ),
        )
        .after_help(
            "Examples:\n  img.qcow2 disk.qcow2 info\n  \
             img.qcow2 disk.qcow2 get virtual_size --text\n  \
             img.qcow2 disk.qcow2 read -o disk.raw\n  \
             img.qcow2 disk.qcow2 read --offset 0 --length 512 | xxd",
        )
}

fn byte_count(id: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(id)
        .value_name("BYTES")
        .help(help)
        .value_parser(super::size::parse)
}

fn key_command(name: &'static str, about: &'static str) -> Cmd {
    Cmd::new(name)
        .about(about)
        .arg(
            Arg::new("key")
                .value_name("KEY")
                .help(format!("One of: {} (or qcow2.<field>)", KEYS.join(", "))),
        )
        .after_help(format!(
            "Examples:\n  img.qcow2 disk.qcow2 {name}\n  \
             img.qcow2 disk.qcow2 {name} virtual_size --text\n  \
             img.qcow2 disk.qcow2 {name} qcow2.compression_type"
        ))
}

fn run(matches: &ArgMatches) -> Result<Outcome, CliError> {
    let image = Path::new(
        matches
            .get_one::<OsString>("image")
            .expect("clap requires the image"),
    );
    let (verb, sub) = matches.subcommand().expect("clap requires a verb");
    match verb {
        "info" | "get" => get(image, sub.get_one::<String>("key").map(String::as_str)),
        "read" => read(
            image,
            sub.get_one::<u64>("offset").copied(),
            sub.get_one::<u64>("length").copied(),
            sub.get_one::<OsString>("output").map(PathBuf::from),
        ),
        "write" => write(
            image,
            *sub.get_one::<u64>("offset")
                .expect("clap requires the offset"),
        ),
        "create" => Err(CliError::not_implemented(
            "create: this library has no QCOW2 image creator",
        )),
        "set" => Err(CliError::not_implemented(
            "set: this library changes no header field",
        )),
        "resize" => Err(CliError::not_implemented(
            "resize: this library cannot resize a QCOW2 image",
        )),
        other => unreachable!("clap knows no verb {other}"),
    }
}

fn qcow2_error(image: &Path, e: qcow2::Error) -> CliError {
    CliError::failed(format!("{}: {e}", image.display()))
}

fn open(image: &Path) -> Result<Qcow2Reader, CliError> {
    Qcow2Reader::open(image).map_err(|e| qcow2_error(image, e))
}

/// The backing file's name as the header records it, not resolved: what
/// `qemu-img info` calls `backing-filename`. The library resolves it
/// against the image's directory to open the chain; this is the name.
fn backing_name(image: &Path, r: &Qcow2Reader) -> Result<Option<String>, CliError> {
    let h = r.header();
    if h.backing_file_size == 0 || h.backing_file_offset == 0 {
        return Ok(None);
    }
    let io = |e: std::io::Error| CliError::failed(format!("{}: {e}", image.display()));
    let mut f = std::fs::File::open(image).map_err(io)?;
    f.seek(SeekFrom::Start(h.backing_file_offset)).map_err(io)?;
    let mut bytes = vec![0u8; h.backing_file_size as usize];
    f.read_exact(&mut bytes).map_err(io)?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

fn compression_name(t: u8) -> Json {
    match t {
        0 => Json::from("zlib"),
        1 => Json::from("zstd"),
        other => Json::from(format!("unknown ({other})")),
    }
}

/// The envelope: the shared keys first, the format's own under `qcow2`.
pub fn envelope(image: &Path, r: &Qcow2Reader) -> Result<Json, CliError> {
    let h = r.header();
    Ok(Json::object([
        ("format", Json::from("qcow2")),
        ("virtual_size", Json::from(r.virtual_size())),
        ("block_size", Json::from(r.cluster_size())),
        ("backing", Json::from(backing_name(image, r)?)),
        // Refcounts left unwritten at the last shutdown (v3's dirty bit).
        ("dirty", Json::from(h.has_dirty_refcounts())),
        (
            "qcow2",
            Json::object([
                ("version", Json::from(h.version)),
                ("cluster_size", Json::from(h.cluster_size)),
                ("cluster_bits", Json::from(h.cluster_bits)),
                ("compression_type", compression_name(h.compression_type)),
                ("refcount_bits", Json::from(1u64 << h.refcount_order.min(6))),
                ("l1_size", Json::from(h.l1_size)),
                ("snapshot_count", Json::from(h.nb_snapshots)),
                ("encrypted", Json::from(h.crypt_method != 0)),
                ("corrupt", Json::from(h.is_corrupt())),
                (
                    "features",
                    Json::object([
                        ("incompatible", Json::from(h.incompatible_features)),
                        ("compatible", Json::from(h.compatible_features)),
                        ("autoclear", Json::from(h.autoclear_features)),
                    ]),
                ),
            ]),
        ),
    ]))
}

fn get(image: &Path, key: Option<&str>) -> Result<Outcome, CliError> {
    let r = open(image)?;
    let all = envelope(image, &r)?;
    let Some(key) = key else {
        return Ok(Outcome::report(all));
    };
    let mut value = Some(&all);
    for part in key.split('.') {
        value = value.and_then(|v| v.get(part));
    }
    let Some(value) = value else {
        return Err(CliError::usage(format!(
            "no key {key:?}; the keys are {} (and qcow2.<field>)",
            KEYS.join(", ")
        )));
    };
    let text = value.to_text();
    Ok(Outcome::report(Json::object([(key, value.clone())])).with_text(text))
}

/// The range a `read` covers: `offset` (default 0) for `length` bytes
/// (default: to the end), refused whole if any of it is past the end, so
/// nothing is written for a range that cannot be served.
fn range(size: u64, offset: Option<u64>, length: Option<u64>) -> Result<(u64, u64), CliError> {
    let offset = offset.unwrap_or(0);
    if offset > size {
        return Err(CliError::failed(format!(
            "--offset {offset} is past the end of the {size}-byte virtual disk"
        )));
    }
    let length = length.unwrap_or(size - offset);
    if offset.checked_add(length).is_none_or(|end| end > size) {
        return Err(CliError::failed(format!(
            "{length} bytes at {offset} run past the end of the {size}-byte virtual disk"
        )));
    }
    Ok((offset, length))
}

/// Stream the guest's bytes. Each chunk is read before it is written, so
/// an image that turns out unreadable part-way stops with status 1; what
/// can be refused up front (no image, a range past the end) is refused
/// before a byte is written. `-o FILE` writes `FILE.partial` and renames
/// it, so FILE is never left half written, and skips runs of zeros so a
/// mostly empty disk makes a sparse file.
fn read(
    image: &Path,
    offset: Option<u64>,
    length: Option<u64>,
    output: Option<PathBuf>,
) -> Result<Outcome, CliError> {
    let r = open(image)?;
    let (offset, length) = range(r.virtual_size(), offset, length)?;
    let mut buf = vec![0u8; CHUNK];
    match output {
        None => {
            let mut out = std::io::stdout().lock();
            let mut at = offset;
            let end = offset + length;
            while at < end {
                let n = CHUNK.min((end - at) as usize);
                r.read_at(at, &mut buf[..n])
                    .map_err(|e| qcow2_error(image, e))?;
                if let Err(e) = out.write_all(&buf[..n]) {
                    // A closed pipe (`| head -c 512`) is the reader's
                    // choice, not a failure.
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        return Ok(Outcome::done());
                    }
                    return Err(CliError::failed(format!("write stdout: {e}")));
                }
                at += n as u64;
            }
            if let Err(e) = out.flush() {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(CliError::failed(format!("write stdout: {e}")));
                }
            }
        }
        Some(dest) => {
            let mut partial = dest.as_os_str().to_owned();
            partial.push(".partial");
            let partial = PathBuf::from(partial);
            let copied = (|| -> Result<(), CliError> {
                let io = |e: std::io::Error| {
                    CliError::failed(format!("write {}: {e}", partial.display()))
                };
                let mut f = std::fs::File::create(&partial).map_err(io)?;
                let mut at = offset;
                let end = offset + length;
                while at < end {
                    let n = CHUNK.min((end - at) as usize);
                    r.read_at(at, &mut buf[..n])
                        .map_err(|e| qcow2_error(image, e))?;
                    if buf[..n].iter().all(|b| *b == 0) {
                        f.seek(SeekFrom::Current(n as i64)).map_err(io)?;
                    } else {
                        f.write_all(&buf[..n]).map_err(io)?;
                    }
                    at += n as u64;
                }
                // A trailing run of zeros was skipped, not written: the
                // length is set, not implied.
                f.set_len(length).map_err(io)?;
                f.sync_all().map_err(io)
            })();
            if let Err(e) = copied {
                let _ = std::fs::remove_file(&partial);
                return Err(e);
            }
            std::fs::rename(&partial, &dest)
                .map_err(|e| CliError::failed(format!("rename to {}: {e}", dest.display())))?;
        }
    }
    Ok(Outcome::done())
}

/// What `write` reads its bytes from.
enum Input {
    /// A regular file redirected onto stdin: its length is known before a
    /// byte is read, so the bounds are checked first and it is streamed.
    File(std::fs::File, u64),
    /// A pipe or a terminal: read whole, at most one byte more than fits,
    /// so input that does not fit is refused before anything is written.
    Buffered(Vec<u8>),
}

/// Stdin as a regular file, when it is one.
#[cfg(unix)]
fn stdin_file() -> Option<std::fs::File> {
    use std::os::fd::AsFd;
    let fd = std::io::stdin().as_fd().try_clone_to_owned().ok()?;
    let file = std::fs::File::from(fd);
    file.metadata().ok().filter(|m| m.is_file()).map(|_| file)
}

/// Elsewhere stdin is always read as a pipe. Measured on Windows: a pipe's
/// handle answers `metadata()` as a file of the bytes queued so far, so a
/// pipe would be taken for a file and its length checked before it was full.
#[cfg(not(unix))]
fn stdin_file() -> Option<std::fs::File> {
    None
}

/// Whether `input` is the file at `image`: an image written from itself
/// grows as it is read, each cluster it allocates landing where the next
/// read comes from.
#[cfg(unix)]
fn is_same_file(input: &std::fs::File, image: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (input.metadata(), std::fs::metadata(image)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_same_file(_input: &std::fs::File, _image: &Path) -> bool {
    false
}

/// A write the library refuses by what the image is (flagged dirty or
/// corrupt, a refcount width it does not write) is a verb it cannot do yet;
/// anything else failed.
fn write_error(image: &Path, e: qcow2::Error) -> CliError {
    match e {
        qcow2::Error::Unsupported(why) => {
            CliError::not_implemented(format!("write: {}: {why}", image.display()))
        }
        other => qcow2_error(image, other),
    }
}

/// Write everything on stdin into the guest at `offset`.
///
/// Everything that can be refused is refused before the first byte is
/// written: an image the library will not write, input that would run past
/// the end of the virtual disk, and the image itself redirected onto stdin.
fn write(image: &Path, offset: u64) -> Result<Outcome, CliError> {
    let r = Qcow2Reader::open_rw(image).map_err(|e| write_error(image, e))?;
    r.header()
        .check_writable()
        .map_err(|e| write_error(image, e))?;
    let size = r.virtual_size();
    if offset > size {
        return Err(CliError::failed(format!(
            "--offset {offset} is past the end of the {size}-byte virtual disk"
        )));
    }
    let room = size - offset;
    let stdin_error = |e: std::io::Error| CliError::failed(format!("read stdin: {e}"));
    let input = match stdin_file() {
        Some(mut file) => {
            if is_same_file(&file, image) {
                return Err(CliError::failed(format!(
                    "{}: stdin is the image being written",
                    image.display()
                )));
            }
            let len = file.metadata().map_err(stdin_error)?.len();
            let at = file.stream_position().map_err(stdin_error)?;
            let n = len.saturating_sub(at);
            if n > room {
                return Err(CliError::failed(format!(
                    "{n} bytes at {offset} run past the end of the {size}-byte virtual disk"
                )));
            }
            Input::File(file, n)
        }
        None => {
            let mut data = Vec::new();
            std::io::stdin()
                .lock()
                .take(room.saturating_add(1))
                .read_to_end(&mut data)
                .map_err(stdin_error)?;
            if data.len() as u64 > room {
                return Err(CliError::failed(format!(
                    "stdin holds more than the {room} bytes from {offset} to the end of the \
                     {size}-byte virtual disk"
                )));
            }
            Input::Buffered(data)
        }
    };
    let written = match input {
        Input::Buffered(data) => {
            for (i, chunk) in data.chunks(CHUNK).enumerate() {
                r.write_at(offset + (i * CHUNK) as u64, chunk)
                    .map_err(|e| write_error(image, e))?;
            }
            data.len() as u64
        }
        Input::File(file, n) => {
            // No further than the length just checked, whatever the file
            // does while it is read.
            let mut file = file.take(n);
            let mut buf = vec![0u8; CHUNK];
            let mut at = offset;
            loop {
                let got = file.read(&mut buf).map_err(stdin_error)?;
                if got == 0 {
                    break;
                }
                r.write_at(at, &buf[..got])
                    .map_err(|e| write_error(image, e))?;
                at += got as u64;
            }
            at - offset
        }
    };
    r.flush().map_err(|e| qcow2_error(image, e))?;
    let report = Json::object([
        ("offset", Json::from(offset)),
        ("bytes", Json::from(written)),
    ]);
    Ok(Outcome::report(report).with_text(format!("wrote {written} bytes at {offset}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_defaults_to_the_whole_disk_and_refuses_the_far_side() {
        assert_eq!(range(1000, None, None).ok(), Some((0, 1000)));
        assert_eq!(range(1000, Some(10), None).ok(), Some((10, 990)));
        assert_eq!(range(1000, None, Some(10)).ok(), Some((0, 10)));
        assert_eq!(range(1000, Some(1000), None).ok(), Some((1000, 0)));
        assert!(range(1000, Some(1001), None).is_err());
        assert!(range(1000, Some(990), Some(11)).is_err());
        assert!(range(1000, Some(u64::MAX), Some(2)).is_err());
    }

    #[test]
    fn compression_types_are_named_and_an_unknown_one_says_so() {
        assert_eq!(compression_name(0), Json::from("zlib"));
        assert_eq!(compression_name(1), Json::from("zstd"));
        assert_eq!(compression_name(7), Json::from("unknown (7)"));
    }
}
