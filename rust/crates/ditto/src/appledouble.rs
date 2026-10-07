//! AppleDouble files (`._name`), which zip archives made on macOS use to
//! carry Finder info, resource forks, and extended attributes.
//!
//! Layout: a 26-byte header (magic `0x00051607`, version `0x00020000`, 16
//! filler bytes, entry count), then 12-byte entry descriptors (id, offset,
//! length). Entry 9 holds 32 bytes of Finder info, which macOS follows with
//! an `ATTR` block of extended attributes; entry 2 holds the resource fork.

const MAGIC: u32 = 0x0005_1607;
const RESOURCE_FORK: u32 = 2;
const FINDER_INFO: u32 = 9;
const FINDER_INFO_LEN: usize = 32;

/// Attributes `ditto --noqtn` doesn't restore, plus the sandbox provenance
/// marker, which describes the machine that made the archive.
const DROPPED_XATTRS: [&str; 2] = ["com.apple.quarantine", "com.apple.provenance"];
/// AppleDouble members carry resource forks, so allow them to be large, but
/// not unbounded, since they're read into memory.
pub(crate) const MAX_METADATA_BYTES: u64 = 256 << 20;

/// Maps `dir/._name` to `dir/name`.
pub(crate) fn sibling(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_str()?.strip_prefix("._")?;
    if name.is_empty() {
        return None;
    }
    Some(path.with_file_name(name))
}

/// Reads at most `limit` bytes, failing when the member is larger.
pub(crate) fn read_bounded(
    reader: &mut impl std::io::Read,
    limit: u64,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Archive member exceeds the metadata size limit",
        ));
    }
    Ok(bytes)
}

/// Applies collected AppleDouble files to the entries they describe.
/// Metadata for an entry that isn't in the archive, or that doesn't parse,
/// is ignored, as `ditto` does.
pub(crate) fn apply(
    writer: &mut russet_fs::TreeWriter,
    metadata: Vec<(std::path::PathBuf, Vec<u8>)>,
) -> std::io::Result<()> {
    for (target, bytes) in metadata {
        let Ok(decoded) = parse(&bytes) else { continue };
        for (name, value) in decoded.xattrs {
            if DROPPED_XATTRS.contains(&name.as_str()) {
                continue;
            }
            match writer.set_xattr(&target, &name, &value) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Metadata decoded from one AppleDouble file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    /// Extended attributes in archive order, including
    /// `com.apple.FinderInfo` and `com.apple.ResourceFork` when present.
    pub xattrs: Vec<(String, Vec<u8>)>,
}

/// Returns true when `bytes` start with the AppleDouble magic number.
pub fn is_apple_double(bytes: &[u8]) -> bool {
    read_u32(bytes, 0) == Some(MAGIC)
}

/// Decodes an AppleDouble file. Zero Finder info is left out, as `ditto`
/// does.
pub fn parse(bytes: &[u8]) -> Result<Metadata, String> {
    let bad = || "Invalid AppleDouble metadata".to_string();
    if !is_apple_double(bytes) {
        return Err(bad());
    }
    let count = read_u16(bytes, 24).ok_or_else(bad)? as usize;
    let mut metadata = Metadata::default();
    let mut fork = None;
    for index in 0..count {
        let at = 26 + index * 12;
        let (id, offset, length) = (
            read_u32(bytes, at).ok_or_else(bad)?,
            read_u32(bytes, at + 4).ok_or_else(bad)? as usize,
            read_u32(bytes, at + 8).ok_or_else(bad)? as usize,
        );
        let data = bytes
            .get(offset..offset.checked_add(length).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        match id {
            FINDER_INFO => {
                let info = data
                    .get(..FINDER_INFO_LEN.min(data.len()))
                    .unwrap_or_default();
                if info.iter().any(|b| *b != 0) {
                    metadata
                        .xattrs
                        .push(("com.apple.FinderInfo".into(), info.to_vec()));
                }
                // The ATTR block follows the Finder info after two bytes of
                // padding; its offsets are relative to the whole file.
                if let Some(header) = data.get(FINDER_INFO_LEN + 2..) {
                    attributes(bytes, header, &mut metadata.xattrs)?;
                }
            }
            RESOURCE_FORK if length > 0 => fork = Some(data.to_vec()),
            _ => {}
        }
    }
    if let Some(fork) = fork {
        metadata
            .xattrs
            .push(("com.apple.ResourceFork".into(), fork));
    }
    Ok(metadata)
}

fn attributes(file: &[u8], header: &[u8], out: &mut Vec<(String, Vec<u8>)>) -> Result<(), String> {
    let bad = || "Invalid AppleDouble attribute block".to_string();
    if header.get(..4) != Some(b"ATTR") {
        return Ok(());
    }
    let count = read_u16(header, 34).ok_or_else(bad)? as usize;
    let mut at = 36;
    for _ in 0..count {
        let offset = read_u32(header, at).ok_or_else(bad)? as usize;
        let length = read_u32(header, at + 4).ok_or_else(bad)? as usize;
        let name_len = *header.get(at + 10).ok_or_else(bad)? as usize;
        let name = header.get(at + 11..at + 11 + name_len).ok_or_else(bad)?;
        let name =
            std::str::from_utf8(name.strip_suffix(&[0]).unwrap_or(name)).map_err(|_| bad())?;
        let value = file
            .get(offset..offset.checked_add(length).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        out.push((name.to_owned(), value.to_vec()));
        // Each entry is padded to a four-byte boundary.
        at = (at + 11 + name_len + 3) & !3;
    }
    Ok(())
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds an AppleDouble file the way `ditto -c -k` does.
    pub(crate) fn build(finder: [u8; 32], attrs: &[(&str, &[u8])], fork: &[u8]) -> Vec<u8> {
        let mut entries = Vec::new();
        for (name, _) in attrs {
            let mut entry = vec![0; 11];
            entry[10] = (name.len() + 1) as u8;
            entry.extend_from_slice(name.as_bytes());
            entry.push(0);
            while !entry.len().is_multiple_of(4) {
                entry.push(0);
            }
            entries.push(entry);
        }
        let header_at = 26 + 2 * 12 + FINDER_INFO_LEN + 2;
        let table_len: usize = entries.iter().map(Vec::len).sum();
        let mut data_at = header_at + 36 + table_len;
        let mut values = Vec::new();
        for (entry, (_, value)) in entries.iter_mut().zip(attrs) {
            entry[..4].copy_from_slice(&(data_at as u32).to_be_bytes());
            entry[4..8].copy_from_slice(&(value.len() as u32).to_be_bytes());
            values.extend_from_slice(value);
            data_at += value.len();
        }
        let fork_at = data_at;
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC.to_be_bytes());
        out.extend_from_slice(&0x0002_0000u32.to_be_bytes());
        out.extend_from_slice(b"Mac OS X        ");
        out.extend_from_slice(&2u16.to_be_bytes());
        let finder_len = fork_at - (26 + 24);
        for (id, offset, length) in [
            (FINDER_INFO, 26 + 24, finder_len),
            (RESOURCE_FORK, fork_at, fork.len()),
        ] {
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(length as u32).to_be_bytes());
        }
        out.extend_from_slice(&finder);
        out.extend_from_slice(&[0, 0]);
        let mut header = vec![0; 36];
        header[..4].copy_from_slice(b"ATTR");
        header[34..36].copy_from_slice(&(attrs.len() as u16).to_be_bytes());
        out.extend_from_slice(&header);
        for entry in entries {
            out.extend_from_slice(&entry);
        }
        out.extend_from_slice(&values);
        out.extend_from_slice(fork);
        out
    }

    #[test]
    fn decodes_attributes_fork_and_finder_info() {
        let mut finder = [0; 32];
        finder[8] = 4;
        let bytes = build(finder, &[("com.example.test", b"hello")], b"RSRC");
        let metadata = parse(&bytes).unwrap();
        assert_eq!(
            metadata.xattrs,
            vec![
                ("com.apple.FinderInfo".to_string(), finder.to_vec()),
                ("com.example.test".to_string(), b"hello".to_vec()),
                ("com.apple.ResourceFork".to_string(), b"RSRC".to_vec()),
            ]
        );
    }

    #[test]
    fn skips_zero_finder_info_and_rejects_truncation() {
        let bytes = build([0; 32], &[], b"");
        assert!(parse(&bytes).unwrap().xattrs.is_empty());
        let full = build([1; 32], &[("a", b"b")], b"");
        assert!(parse(&full[..full.len() - 1]).is_err());
    }
}
