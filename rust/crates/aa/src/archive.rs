//! Apple Archive entries. Each starts with `AA01` (or the older `YAA1`) and
//! a little-endian 16-bit header size, then fields: a 3-letter key and a
//! subtype letter that sets the value's encoding. Blob fields record only
//! their size in the header; their bytes follow the header in field order.

use crate::stream::invalid;
use russet_fs::{clean_relative, Limits, SkippedXattr, TreeWriter};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

/// Largest blob read into memory other than file data: extended attributes
/// and access control lists.
const MAX_METADATA_BLOB: u64 = 64 << 20;

/// One field's value.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Flag,
    Uint(u64),
    String(Vec<u8>),
    Hash,
    Time,
    /// A blob of this many bytes, after the header.
    Blob(u64),
}

/// One entry's header.
#[derive(Debug, Default)]
pub(crate) struct Header {
    fields: Vec<([u8; 3], Value)>,
}

impl Header {
    fn uint(&self, key: &[u8; 3]) -> Option<u64> {
        self.fields.iter().find_map(|(k, v)| match v {
            Value::Uint(n) if k == key => Some(*n),
            _ => None,
        })
    }

    fn string(&self, key: &[u8; 3]) -> Option<&[u8]> {
        self.fields.iter().find_map(|(k, v)| match v {
            Value::String(s) if k == key => Some(s.as_slice()),
            _ => None,
        })
    }

    /// The entry type, from the `TYP` field: `F` (file), `D` (folder), `L`
    /// (symlink), and others.
    pub(crate) fn kind(&self) -> Option<u8> {
        self.uint(b"TYP").map(|t| t as u8)
    }

    pub(crate) fn path(&self) -> io::Result<PathBuf> {
        let path = self.string(b"PAT").unwrap_or_default();
        clean_relative(Path::new(&OsString::from_vec(path.to_vec())))
    }
}

fn read_uint(reader: &mut impl Read, size: usize) -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes[..size])?;
    Ok(u64::from_le_bytes(bytes))
}

/// Reads the next entry's header, or `None` at the end of the archive.
pub(crate) fn header(reader: &mut impl Read) -> io::Result<Option<Header>> {
    let mut start = [0u8; 6];
    match reader.read(&mut start[..1])? {
        0 => return Ok(None),
        _ => reader.read_exact(&mut start[1..])?,
    }
    if &start[..4] != b"AA01" && &start[..4] != b"YAA1" {
        return Err(invalid("Invalid Apple Archive entry header"));
    }
    let size = u16::from_le_bytes([start[4], start[5]]) as usize;
    if size < 6 {
        return Err(invalid("Invalid Apple Archive entry header size"));
    }
    let mut bytes = vec![0u8; size - 6];
    reader.read_exact(&mut bytes)?;
    let mut fields = Vec::new();
    let mut cursor = bytes.as_slice();
    while !cursor.is_empty() {
        let mut tag = [0u8; 4];
        cursor.read_exact(&mut tag)?;
        let key = [tag[0], tag[1], tag[2]];
        let value = match tag[3] {
            b'*' => Value::Flag,
            b'1' => Value::Uint(read_uint(&mut cursor, 1)?),
            b'2' => Value::Uint(read_uint(&mut cursor, 2)?),
            b'4' => Value::Uint(read_uint(&mut cursor, 4)?),
            b'8' => Value::Uint(read_uint(&mut cursor, 8)?),
            b'A' => Value::Blob(read_uint(&mut cursor, 2)?),
            b'B' => Value::Blob(read_uint(&mut cursor, 4)?),
            b'C' => Value::Blob(read_uint(&mut cursor, 8)?),
            b'P' => {
                let length = read_uint(&mut cursor, 2)? as usize;
                let mut value = vec![0u8; length];
                cursor.read_exact(&mut value)?;
                Value::String(value)
            }
            b'S' => {
                read_uint(&mut cursor, 8)?;
                Value::Time
            }
            b'T' => {
                read_uint(&mut cursor, 8)?;
                read_uint(&mut cursor, 4)?;
                Value::Time
            }
            hash @ (b'F' | b'G' | b'H' | b'I' | b'J') => {
                let size = match hash {
                    b'F' => 4,
                    b'G' => 20,
                    b'H' => 32,
                    b'I' => 48,
                    _ => 64,
                };
                let mut value = [0u8; 64];
                cursor.read_exact(&mut value[..size])?;
                Value::Hash
            }
            other => {
                return Err(invalid(format!(
                    "Unknown Apple Archive field type '{}{}'",
                    String::from_utf8_lossy(&key),
                    other as char
                )))
            }
        };
        fields.push((key, value));
    }
    Ok(Some(Header { fields }))
}

/// Parses an `XAT` blob: entries of a 32-bit size (counting itself), a
/// NUL-terminated name, and the value.
fn xattrs(mut blob: &[u8]) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    while !blob.is_empty() {
        let size = blob
            .get(..4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            .ok_or_else(|| invalid("Truncated extended attribute"))?;
        let entry = blob
            .get(4..size.max(4))
            .filter(|_| size >= 4)
            .ok_or_else(|| invalid("Truncated extended attribute"))?;
        let nul = entry
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| invalid("Unterminated extended attribute name"))?;
        let name = String::from_utf8(entry[..nul].to_vec())
            .map_err(|_| invalid("Extended attribute name isn't UTF-8"))?;
        out.push((name, entry[nul + 1..].to_vec()));
        blob = &blob[size..];
    }
    Ok(out)
}

/// Calls `visit` with each blob of an entry in order, as a reader limited
/// to the blob's size, then skips whatever `visit` left unread.
fn blobs<R: Read>(
    reader: &mut R,
    header: &Header,
    mut visit: impl FnMut(&[u8; 3], u64, &mut io::Take<&mut R>) -> io::Result<()>,
) -> io::Result<()> {
    for (key, value) in &header.fields {
        if let Value::Blob(size) = value {
            let mut blob = (&mut *reader).take(*size);
            visit(key, *size, &mut blob)?;
            io::copy(&mut blob, &mut io::sink())?;
            if blob.limit() != 0 {
                return Err(invalid("Truncated Apple Archive"));
            }
        }
    }
    Ok(())
}

fn read_metadata(blob: &mut impl Read, size: u64) -> io::Result<Vec<u8>> {
    if size > MAX_METADATA_BLOB {
        return Err(invalid("Apple Archive metadata blob is too large"));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    blob.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Extracts an uncompressed Apple Archive stream like `aa extract`: files,
/// folders, symlinks, and hard links with their modes and extended
/// attributes. Other entry types, such as devices and FIFOs, are skipped.
pub(crate) fn extract(
    mut reader: impl Read,
    destination: &Path,
    limits: Limits,
) -> io::Result<Vec<SkippedXattr>> {
    let mut writer = TreeWriter::open(destination, limits)?;
    // The first path written for each hard link cluster.
    let mut clusters: HashMap<u64, PathBuf> = HashMap::new();
    while let Some(header) = header(&mut reader)? {
        let path = header.path()?;
        let mode = header.uint(b"MOD").unwrap_or(0o644) as u32;
        let mut written = true;
        match header.kind() {
            Some(b'D') => writer.create_dir(&path, Some(mode))?,
            Some(b'L') => {
                let target = header
                    .string(b"LNK")
                    .ok_or_else(|| invalid("Apple Archive symlink has no target"))?;
                writer.symlink(&path, &OsString::from_vec(target.to_vec()))?;
            }
            Some(b'F') => {
                let cluster = header.uint(b"HLC");
                if let Some(first) = cluster.and_then(|c| clusters.get(&c)) {
                    writer.hard_link(&path, first)?;
                    written = false;
                } else {
                    let mut wrote_data = false;
                    let mut xattr_list = Vec::new();
                    blobs(&mut reader, &header, |key, size, blob| {
                        match key {
                            b"DAT" => {
                                writer.write_file(&path, blob, mode)?;
                                wrote_data = true;
                            }
                            b"XAT" => xattr_list = xattrs(&read_metadata(blob, size)?)?,
                            _ => {}
                        }
                        Ok(())
                    })?;
                    if !wrote_data {
                        writer.write_file(&path, io::empty(), mode)?;
                    }
                    for (name, value) in xattr_list {
                        writer.set_xattr(&path, &name, &value)?;
                    }
                    if let Some(cluster) = cluster {
                        clusters.insert(cluster, path.clone());
                    }
                    continue;
                }
            }
            _ => written = false,
        }
        let mut xattr_list = Vec::new();
        blobs(&mut reader, &header, |key, size, blob| {
            if key == b"XAT" && written {
                xattr_list = xattrs(&read_metadata(blob, size)?)?;
            }
            Ok(())
        })?;
        for (name, value) in xattr_list {
            writer.set_xattr(&path, &name, &value)?;
        }
    }
    writer.finish()
}

/// Reads one regular file without extracting anything else.
pub(crate) fn read_member(
    mut reader: impl Read,
    target: &Path,
    max_bytes: u64,
) -> io::Result<Option<Vec<u8>>> {
    while let Some(header) = header(&mut reader)? {
        let wanted = header.kind() == Some(b'F') && header.path()? == target;
        let mut found = None;
        blobs(&mut reader, &header, |key, size, blob| {
            if wanted && key == b"DAT" {
                if size > max_bytes {
                    return Err(invalid(format!(
                        "{} is larger than {max_bytes} bytes",
                        target.display()
                    )));
                }
                let mut bytes = Vec::with_capacity(size as usize);
                blob.read_to_end(&mut bytes)?;
                found = Some(bytes);
            }
            Ok(())
        })?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}
