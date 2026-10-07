//! `ditto -x` for cpio archives: the portable `odc` format (`070707`) used by
//! macOS package payloads, and the `newc` format (`070701`).

use crate::appledouble::{self, read_bounded, sibling, MAX_METADATA_BYTES};
use russet_fs::{clean_relative, Limits, SkippedXattr, TreeWriter};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;
const TRAILER: &[u8] = b"TRAILER!!!";
const MAX_NAME_BYTES: u64 = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Odc,
    Newc,
}

struct Header {
    format: Format,
    dev: u64,
    ino: u64,
    mode: u32,
    nlink: u64,
    name_size: u64,
    file_size: u64,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn field(bytes: &[u8], radix: u32) -> io::Result<u64> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("Invalid cpio header"))?;
    u64::from_str_radix(text, radix).map_err(|_| invalid("Invalid cpio header"))
}

fn header(reader: &mut impl Read) -> io::Result<Header> {
    let mut magic = [0; 6];
    reader.read_exact(&mut magic)?;
    match &magic {
        b"070707" => {
            let mut rest = [0; 70];
            reader.read_exact(&mut rest)?;
            Ok(Header {
                format: Format::Odc,
                dev: field(&rest[0..6], 8)?,
                ino: field(&rest[6..12], 8)?,
                mode: field(&rest[12..18], 8)? as u32,
                nlink: field(&rest[30..36], 8)?,
                name_size: field(&rest[53..59], 8)?,
                file_size: field(&rest[59..70], 8)?,
            })
        }
        b"070701" | b"070702" => {
            let mut rest = [0; 104];
            reader.read_exact(&mut rest)?;
            let hex = |i: usize| field(&rest[i * 8..i * 8 + 8], 16);
            Ok(Header {
                format: Format::Newc,
                ino: hex(0)?,
                mode: hex(1)? as u32,
                nlink: hex(4)?,
                file_size: hex(6)?,
                dev: (hex(7)? << 32) | hex(8)?,
                name_size: hex(11)?,
            })
        }
        _ => Err(invalid("Not a cpio archive, or an unsupported cpio format")),
    }
}

/// Skips the padding `newc` uses to align to four bytes.
fn align(reader: &mut impl Read, format: Format, consumed: u64) -> io::Result<()> {
    if format == Format::Newc {
        let pad = (4 - consumed % 4) % 4;
        io::copy(&mut reader.take(pad), &mut io::sink())?;
    }
    Ok(())
}

/// Skips any unread member data, checks it was all present, and skips the
/// alignment padding before the next header.
fn continue_after<R: Read>(
    mut data: io::Take<&mut R>,
    format: Format,
    size: u64,
) -> io::Result<()> {
    io::copy(&mut data, &mut io::sink())?;
    if data.limit() != 0 {
        return Err(invalid("Truncated cpio archive"));
    }
    align(data.into_inner(), format, size)
}

/// Reads the regular file at `target` (a cleaned relative path) without
/// extracting anything. Returns `None` when the archive has no such file.
pub(crate) fn read_member(
    mut reader: impl Read,
    target: &Path,
    max_bytes: u64,
) -> io::Result<Option<Vec<u8>>> {
    loop {
        let header = header(&mut reader)?;
        if header.name_size == 0 || header.name_size > MAX_NAME_BYTES {
            return Err(invalid("Invalid cpio member name length"));
        }
        let mut name = Vec::new();
        (&mut reader)
            .take(header.name_size)
            .read_to_end(&mut name)?;
        if name.len() as u64 != header.name_size {
            return Err(invalid("Truncated cpio archive"));
        }
        let fixed = if header.format == Format::Odc {
            76
        } else {
            110
        };
        align(&mut reader, header.format, fixed + header.name_size)?;
        let name = name.strip_suffix(&[0]).unwrap_or(&name).to_vec();
        if name == TRAILER {
            return Ok(None);
        }
        let path = clean_relative(Path::new(&OsString::from_vec(name)))?;
        let mut data = (&mut reader).take(header.file_size);
        if path == target && header.mode & S_IFMT == S_IFREG && header.file_size > 0 {
            if header.file_size > max_bytes {
                return Err(invalid(format!(
                    "{} is larger than {max_bytes} bytes",
                    target.display()
                )));
            }
            let mut bytes = Vec::with_capacity(header.file_size as usize);
            data.read_to_end(&mut bytes)?;
            if bytes.len() as u64 != header.file_size {
                return Err(invalid("Truncated cpio archive"));
            }
            return Ok(Some(bytes));
        }
        continue_after(data, header.format, header.file_size)?;
    }
}

pub(crate) fn extract(
    mut reader: impl Read,
    destination: &Path,
    limits: Limits,
    apple_double: bool,
) -> io::Result<Vec<SkippedXattr>> {
    let mut writer = TreeWriter::open(destination, limits)?;
    // Hard links: the first member written for an inode, and names still
    // waiting for data (newc stores the data with the last link).
    let mut written: HashMap<(u64, u64), PathBuf> = HashMap::new();
    let mut waiting: HashMap<(u64, u64), Vec<PathBuf>> = HashMap::new();
    let mut metadata = Vec::new();
    loop {
        let header = header(&mut reader)?;
        if header.name_size == 0 || header.name_size > MAX_NAME_BYTES {
            return Err(invalid("Invalid cpio member name length"));
        }
        let mut name = Vec::new();
        (&mut reader)
            .take(header.name_size)
            .read_to_end(&mut name)?;
        if name.len() as u64 != header.name_size {
            return Err(invalid("Truncated cpio archive"));
        }
        let fixed = if header.format == Format::Odc {
            76
        } else {
            110
        };
        align(&mut reader, header.format, fixed + header.name_size)?;
        let name = name.strip_suffix(&[0]).unwrap_or(&name).to_vec();
        if name == TRAILER {
            break;
        }
        let path = clean_relative(Path::new(&OsString::from_vec(name)))?;
        let mut data = (&mut reader).take(header.file_size);
        let mode = header.mode & 0o7777;
        match header.mode & S_IFMT {
            _ if path.as_os_str().is_empty() => {
                io::copy(&mut data, &mut io::sink())?;
            }
            S_IFDIR => writer.create_dir(&path, Some(mode))?,
            S_IFLNK => {
                let mut target = Vec::new();
                data.read_to_end(&mut target)?;
                writer.symlink(&path, &OsString::from_vec(target))?;
            }
            S_IFREG => {
                // `ditto -c` stores metadata as `._name` members that reuse the
                // described file's inode number. They're merged back only when
                // they really are AppleDouble.
                let mut buffered = None;
                if let Some(target) = sibling(&path).filter(|_| apple_double) {
                    let bytes = read_bounded(&mut data, MAX_METADATA_BYTES)?;
                    if appledouble::is_apple_double(&bytes) {
                        metadata.push((target, bytes));
                        continue_after(data, header.format, header.file_size)?;
                        continue;
                    }
                    buffered = Some(bytes);
                }
                let key = (header.dev, header.ino);
                if header.nlink > 1 && written.contains_key(&key) {
                    // odc repeats the data for every link; keep one copy.
                    writer.hard_link(&path, &written[&key])?;
                } else if header.nlink > 1 && header.file_size == 0 {
                    waiting.entry(key).or_default().push(path);
                } else {
                    match buffered {
                        Some(bytes) => writer.write_file(&path, bytes.as_slice(), mode)?,
                        None => writer.write_file(&path, &mut data, mode)?,
                    };
                    if header.nlink > 1 {
                        for other in waiting.remove(&key).unwrap_or_default() {
                            writer.hard_link(&other, &path)?;
                        }
                        written.insert(key, path);
                    }
                }
            }
            // Devices, FIFOs, and sockets aren't extracted.
            _ => {}
        }
        continue_after(data, header.format, header.file_size)?;
    }
    // Links whose data never arrived are empty files.
    for paths in waiting.into_values() {
        let mut paths = paths.into_iter();
        if let Some(first) = paths.next() {
            writer.write_file(&first, io::empty(), 0o644)?;
            for other in paths {
                writer.hard_link(&other, &first)?;
            }
        }
    }
    appledouble::apply(&mut writer, metadata)?;
    writer.finish()
}
