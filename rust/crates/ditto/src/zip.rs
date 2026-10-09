//! `ditto -x -k`: zip extraction that keeps Unix modes, symlinks, and the
//! metadata macOS stores in AppleDouble (`._name`) members.

use crate::appledouble::{self, read_bounded, sibling, MAX_LINK_BYTES, MAX_METADATA_BYTES};
use russet_fs::{clean_relative, Limits, SkippedXattr, TreeWriter};
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

enum Role {
    /// A normal member, extracted to this path.
    Content(PathBuf),
    /// AppleDouble metadata for this path.
    Metadata(PathBuf),
    /// A member `ditto` ignores, such as a `__MACOSX` directory.
    Ignored,
}

fn role(name: &str) -> io::Result<Role> {
    let trimmed = name.trim_end_matches('/');
    if let Some(rest) = trimmed
        .strip_prefix("__MACOSX/")
        .or_else(|| (trimmed == "__MACOSX").then_some(""))
    {
        if name.ends_with('/') || rest.is_empty() {
            return Ok(Role::Ignored);
        }
        let path = clean_relative(Path::new(rest))?;
        return Ok(match sibling(&path) {
            Some(target) => Role::Metadata(target),
            None => Role::Ignored,
        });
    }
    Ok(Role::Content(clean_relative(Path::new(name))?))
}

pub(crate) fn extract(
    source: &Path,
    destination: &Path,
    limits: Limits,
) -> io::Result<Vec<SkippedXattr>> {
    let mut archive = zip::ZipArchive::new(File::open(source)?).map_err(zip_error)?;
    // Check every name before writing anything.
    let mut roles = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index).map_err(zip_error)?;
        roles.push(role(&name(&entry))?);
    }
    let mut writer = TreeWriter::open(destination, limits)?;
    let mut metadata = appledouble::Pending::default();
    for (index, role) in roles.into_iter().enumerate() {
        let mut entry = archive.by_index(index).map_err(zip_error)?;
        let mode = entry.unix_mode();
        match role {
            Role::Ignored => {}
            Role::Metadata(target) => {
                let bytes = read_bounded(&mut entry, MAX_METADATA_BYTES)?;
                metadata.add(&mut writer, target, bytes)?;
            }
            Role::Content(path) => {
                let is_dir = entry.is_dir() || mode.is_some_and(|m| m & S_IFMT == S_IFDIR);
                if is_dir {
                    writer.create_dir(&path, Some(mode.unwrap_or(0o755) & 0o7777))?;
                } else if mode.is_some_and(|m| m & S_IFMT == S_IFLNK) {
                    let target = read_bounded(&mut entry, MAX_LINK_BYTES)?;
                    writer.symlink(&path, &OsString::from_vec(target))?;
                } else if let Some(target) = sibling(&path) {
                    // An in-place `._name` member is metadata only when it
                    // really is AppleDouble; otherwise it's an ordinary file.
                    let bytes = read_bounded(&mut entry, MAX_METADATA_BYTES)?;
                    if appledouble::is_apple_double(&bytes) {
                        metadata.add(&mut writer, target, bytes)?;
                    } else {
                        writer.write_file(&path, bytes.as_slice(), mode.unwrap_or(0o644))?;
                    }
                } else {
                    writer.write_file(&path, &mut entry, mode.unwrap_or(0o644))?;
                }
            }
        }
    }
    metadata.finish(&mut writer)?;
    writer.finish()
}

/// The member's name as `ditto` reads it. Without the UTF-8 flag, the zip
/// format says names are CP437, but `ditto` and Archive Utility read the
/// bytes as UTF-8, and macOS zip tools often leave the flag unset. CP437 is
/// only the fallback for names that aren't valid UTF-8.
fn name<'a>(entry: &'a zip::read::ZipFile<'_>) -> std::borrow::Cow<'a, str> {
    match std::str::from_utf8(entry.name_raw()) {
        Ok(name) => name.into(),
        Err(_) => entry.name().into(),
    }
}

fn zip_error(error: zip::result::ZipError) -> io::Error {
    match error {
        zip::result::ZipError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}
