//! `tar -x` as macOS runs it: bsdtar merges `._name` AppleDouble members
//! back into the extended attributes of `name` instead of extracting them
//! as files. Archives made with macOS `tar` carry them, and an app's code
//! signature doesn't seal them (QuickBooks ships like this).

use crate::appledouble::{self, read_bounded, sibling, MAX_LINK_BYTES, MAX_METADATA_BYTES};
use russet_fs::{clean_relative, Limits, SkippedXattr, TreeWriter};
use std::ffi::OsString;
use std::io::{self, Read};
use std::os::unix::ffi::OsStringExt;
use std::path::Path;

pub(crate) fn extract(
    reader: impl Read,
    destination: &Path,
    limits: Limits,
) -> io::Result<Vec<SkippedXattr>> {
    let mut archive = tar::Archive::new(reader);
    let mut writer = TreeWriter::open(destination, limits)?;
    let mut metadata = appledouble::Pending::default();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = clean_relative(&entry.path()?)?;
        if path.as_os_str().is_empty() {
            continue;
        }
        let mode = entry.header().mode()? & 0o7777;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            writer.create_dir(&path, Some(mode))?;
        } else if kind.is_symlink() {
            let target = entry
                .link_name_bytes()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "symlink has no target"))?
                .into_owned();
            if target.len() as u64 > MAX_LINK_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "symlink target is too long",
                ));
            }
            writer.symlink(&path, &OsString::from_vec(target))?;
        } else if kind.is_hard_link() {
            let target = entry
                .link_name()?
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "hard link has no target")
                })?
                .into_owned();
            writer.hard_link(&path, &clean_relative(&target)?)?;
        } else if kind.is_file() || kind == tar::EntryType::Continuous {
            if let Some(target) = sibling(&path) {
                let bytes = read_bounded(&mut entry, MAX_METADATA_BYTES)?;
                if appledouble::is_apple_double(&bytes) {
                    metadata.add(&mut writer, target, bytes)?;
                } else {
                    writer.write_file(&path, bytes.as_slice(), mode)?;
                }
            } else {
                writer.write_file(&path, &mut entry, mode)?;
            }
        }
        // Devices, FIFOs, and sockets aren't extracted; long-name and pax
        // headers are applied by the reader.
    }
    metadata.finish(&mut writer)?;
    writer.finish()
}
