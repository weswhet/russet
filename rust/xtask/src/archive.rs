//! Deterministic distribution archives shared by `package` and `promote`.

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};

/// Archive members keyed by their path below the archive root, with the file
/// contents and permission bits. A `BTreeMap` keeps members in sorted order.
pub type Entries = BTreeMap<String, (Vec<u8>, u32)>;

/// Supported targets with the executable format and machine type that the
/// packaged binaries must declare.
pub const TARGETS: [(&str, Format, u32); 5] = [
    ("aarch64-apple-darwin", Format::MachO, 0x0100_000C),
    ("x86_64-apple-darwin", Format::MachO, 0x0100_0007),
    ("x86_64-unknown-linux-gnu", Format::Elf, 62),
    ("x86_64-pc-windows-msvc", Format::Pe, 0x8664),
    ("x86_64-pc-windows-gnu", Format::Pe, 0x8664),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    MachO,
    Elf,
    Pe,
}

pub fn is_target(target: &str) -> bool {
    TARGETS.iter().any(|(name, _, _)| *name == target)
}

pub fn is_windows(target: &str) -> bool {
    target.contains("windows")
}

pub fn is_apple(target: &str) -> bool {
    target.contains("apple")
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Check that an executable's header matches the target's format and
/// architecture, so a binary built for the wrong target is never packaged.
pub fn verify_binary(data: &[u8], target: &str) -> Result<(), String> {
    let (_, format, architecture) = TARGETS
        .iter()
        .find(|(name, _, _)| *name == target)
        .ok_or_else(|| format!("Unsupported target {target}"))?;
    let valid = match format {
        Format::MachO => {
            data.len() >= 8
                && data[..4] == *b"\xcf\xfa\xed\xfe"
                && u32_at(data, 4) == Some(*architecture)
        }
        Format::Elf => {
            data.len() >= 20
                && data[..6] == *b"\x7fELF\x02\x01"
                && u16_at(data, 18) == Some(*architecture as u16)
        }
        Format::Pe => {
            data.len() >= 64
                && data[..2] == *b"MZ"
                && u32_at(data, 60).is_some_and(|offset| {
                    let offset = offset as usize;
                    data.len() >= offset + 6
                        && data[offset..offset + 4] == *b"PE\0\0"
                        && u16_at(data, offset + 4) == Some(*architecture as u16)
                })
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("Executable does not match target {target}"))
    }
}

/// Write `entries` below `root` as a ZIP archive when `path` ends in `.zip`,
/// and as a gzip-compressed tar archive otherwise. Timestamps and ownership
/// are fixed, so the same entries always produce the same archive.
pub fn write_archive(path: &Path, root: &str, entries: &Entries) -> Result<(), String> {
    let data = if path.extension().is_some_and(|extension| extension == "zip") {
        zip_bytes(root, entries)?
    } else {
        tar_gz_bytes(root, entries)?
    };
    fs::write(path, data).map_err(|error| format!("{}: {error}", path.display()))
}

fn zip_bytes(root: &str, entries: &Entries) -> Result<Vec<u8>, String> {
    let error = |error: zip::result::ZipError| format!("Could not write ZIP archive: {error}");
    let time = zip::DateTime::from_date_and_time(2000, 1, 1, 0, 0, 0)
        .map_err(|error| format!("Invalid archive timestamp: {error}"))?;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, (data, mode)) in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(time)
            .unix_permissions(*mode);
        writer
            .start_file(format!("{root}/{name}"), options)
            .map_err(error)?;
        writer
            .write_all(data)
            .map_err(|error| format!("Could not write ZIP archive: {error}"))?;
    }
    Ok(writer.finish().map_err(error)?.into_inner())
}

fn tar_gz_bytes(root: &str, entries: &Entries) -> Result<Vec<u8>, String> {
    let tar = tar_bytes(root, entries)?;
    let mut encoder = flate2::GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(&tar)
        .and_then(|_| encoder.finish())
        .map_err(|error| format!("Could not compress archive: {error}"))
}

const BLOCK: usize = 512;
const RECORD: usize = BLOCK * 20;

/// Build a POSIX tar stream in the layout that Python's `tarfile` writes:
/// ustar headers with zero ownership and timestamps, a pax `path` record for
/// names that don't fit the 100-byte field, and padding to a 10 KiB record.
fn tar_bytes(root: &str, entries: &Entries) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    for (name, (data, mode)) in entries {
        let member = format!("{root}/{name}");
        if member.len() > 100 || !member.is_ascii() {
            let records = pax_record("path", &member);
            output.extend(ustar_header(
                "././@PaxHeader",
                0,
                records.len() as u64,
                b'x',
            )?);
            push_padded(&mut output, &records);
        }
        output.extend(ustar_header(&member, *mode, data.len() as u64, b'0')?);
        push_padded(&mut output, data);
    }
    output.resize(output.len() + BLOCK * 2, 0);
    let remainder = output.len() % RECORD;
    if remainder > 0 {
        output.resize(output.len() + RECORD - remainder, 0);
    }
    Ok(output)
}

fn push_padded(output: &mut Vec<u8>, data: &[u8]) {
    output.extend_from_slice(data);
    let remainder = data.len() % BLOCK;
    if remainder > 0 {
        output.resize(output.len() + BLOCK - remainder, 0);
    }
}

/// A pax record is `LENGTH KEYWORD=VALUE\n`, where LENGTH counts the whole
/// record including its own digits.
fn pax_record(keyword: &str, value: &str) -> Vec<u8> {
    let base = keyword.len() + value.len() + 3;
    let mut length = base;
    loop {
        let next = base + length.to_string().len();
        if next == length {
            break;
        }
        length = next;
    }
    format!("{length} {keyword}={value}\n").into_bytes()
}

fn ustar_header(name: &str, mode: u32, size: u64, kind: u8) -> Result<[u8; BLOCK], String> {
    let mut header = [0u8; BLOCK];
    // Like Python, store non-ASCII bytes as `?` and truncate to the field;
    // the preceding pax record carries the full name.
    let ascii: Vec<u8> = name
        .chars()
        .map(|c| if c.is_ascii() { c as u8 } else { b'?' })
        .collect();
    let length = ascii.len().min(100);
    header[..length].copy_from_slice(&ascii[..length]);
    octal(&mut header[100..108], u64::from(mode & 0o7777))?;
    octal(&mut header[108..116], 0)?;
    octal(&mut header[116..124], 0)?;
    octal(&mut header[124..136], size)?;
    octal(&mut header[136..148], 0)?;
    header[148..156].copy_from_slice(b"        ");
    header[156] = kind;
    header[257..265].copy_from_slice(b"ustar\x0000");
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..155].copy_from_slice(format!("{checksum:06o}\0").as_bytes());
    Ok(header)
}

fn octal(field: &mut [u8], value: u64) -> Result<(), String> {
    let digits = field.len() - 1;
    let text = format!("{value:0digits$o}");
    if text.len() > digits {
        return Err(format!("Archive field value {value} is too large"));
    }
    field[..digits].copy_from_slice(text.as_bytes());
    field[digits] = 0;
    Ok(())
}

/// One archive member as stored: its full path, contents, the mode bits from
/// its header, and whether it's a regular file.
pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
    pub mode: u32,
    pub regular: bool,
}

/// Read every member of a ZIP archive without extracting it. Modes come from
/// the high 16 bits of each central directory entry's external attributes.
pub fn read_zip(data: &[u8]) -> Result<Vec<Member>, String> {
    let error = |error: zip::result::ZipError| format!("Could not read ZIP archive: {error}");
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).map_err(error)?;
    let mut members = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(error)?;
        let offset = file.central_header_start() as usize + 38;
        let attributes = u32_at(data, offset).ok_or("Could not read ZIP archive: truncated")?;
        let mode = attributes >> 16;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|error| format!("Could not read ZIP archive: {error}"))?;
        members.push(Member {
            name: file.name().to_owned(),
            data: contents,
            mode: mode & 0o7777,
            regular: mode & 0o170000 == 0o100000,
        });
    }
    Ok(members)
}

/// Read every member of a gzip-compressed tar archive without extracting it.
pub fn read_tar_gz(data: &[u8]) -> Result<Vec<Member>, String> {
    let error = |error: std::io::Error| format!("Could not read tar archive: {error}");
    let mut archive = tar::Archive::new(flate2::read::MultiGzDecoder::new(data));
    let mut members = Vec::new();
    for entry in archive.entries().map_err(error)? {
        let mut entry = entry.map_err(error)?;
        let name = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| "Could not read tar archive: member name is not UTF-8".to_owned())?;
        let kind = entry.header().entry_type();
        let mode = entry.header().mode().map_err(error)?;
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents).map_err(error)?;
        members.push(Member {
            name,
            data: contents,
            mode,
            regular: kind.is_file() || kind == tar::EntryType::Continuous,
        });
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pax_record_length_includes_its_own_digits() {
        let record = pax_record("path", &"a".repeat(120));
        let text = String::from_utf8(record.clone()).unwrap();
        let (length, _) = text.split_once(' ').unwrap();
        assert_eq!(length.parse::<usize>().unwrap(), record.len());
        // 9 + 3 = 12 characters plus two length digits make 14.
        assert_eq!(pax_record("path", "abcde"), b"14 path=abcde\n");
    }

    #[test]
    fn long_and_short_names_round_trip_through_tar() {
        let mut entries = Entries::new();
        entries.insert("short".into(), (b"one".to_vec(), 0o644));
        entries.insert(
            format!("{}/name", "d".repeat(120)),
            (b"two".to_vec(), 0o755),
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.tar.gz");
        write_archive(&path, "root", &entries).unwrap();
        let data = fs::read(&path).unwrap();
        assert_eq!(&data[..10], b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff");
        let members = read_tar_gz(&data).unwrap();
        let read: Vec<_> = members
            .iter()
            .map(|m| (m.name.clone(), m.data.clone(), m.mode, m.regular))
            .collect();
        assert_eq!(
            read,
            vec![
                (
                    format!("root/{}/name", "d".repeat(120)),
                    b"two".to_vec(),
                    0o755,
                    true
                ),
                ("root/short".into(), b"one".to_vec(), 0o644, true),
            ]
        );
        assert_eq!(tar_bytes("root", &entries).unwrap().len() % RECORD, 0);
    }

    #[test]
    fn zip_members_keep_unix_modes() {
        let mut entries = Entries::new();
        entries.insert("bin/tool.exe".into(), (b"MZ".to_vec(), 0o755));
        entries.insert("README.md".into(), (b"text".to_vec(), 0o644));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.zip");
        write_archive(&path, "root", &entries).unwrap();
        let members = read_zip(&fs::read(&path).unwrap()).unwrap();
        let read: Vec<_> = members
            .iter()
            .map(|m| (m.name.as_str(), m.mode, m.regular))
            .collect();
        assert_eq!(
            read,
            vec![
                ("root/README.md", 0o644, true),
                ("root/bin/tool.exe", 0o755, true)
            ]
        );
    }

    #[test]
    fn archives_are_reproducible() {
        let mut entries = Entries::new();
        entries.insert("a".into(), (vec![1; 700], 0o644));
        let directory = tempfile::tempdir().unwrap();
        for name in ["one.tar.gz", "two.tar.gz", "one.zip", "two.zip"] {
            write_archive(&directory.path().join(name), "root", &entries).unwrap();
        }
        let read = |name: &str| fs::read(directory.path().join(name)).unwrap();
        assert_eq!(read("one.tar.gz"), read("two.tar.gz"));
        assert_eq!(read("one.zip"), read("two.zip"));
    }
}
