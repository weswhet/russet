//! `hdiutil create -srcfolder`: an HFS+ disk image from a folder.

use crate::image::invalid;
use fstool::block::file::FileBackend;
use fstool::fs::hfs_plus::{FormatOpts, HfsPlus};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

/// What to create, using `hdiutil create`'s names.
pub struct CreateOptions<'a> {
    /// `-fs`: `HFS+`, `Journaled HFS+`, `APFS`, or `Case-insensitive APFS`.
    /// APFS volumes are written as HFS+; see [`create`].
    pub filesystem: &'a str,
    /// `-format`: `UDZO`, `UDBZ`, `ULFO`, or `UDRO`.
    pub format: &'a str,
    /// `-imagekey zlib-level=N` for `UDZO`.
    pub zlib_level: u32,
    /// `-megabytes`; sized to fit the contents when `None`.
    pub megabytes: Option<u64>,
}

const BLOCK: u64 = 4096;
/// Seconds from 1904-01-01 to 1970-01-01.
const HFS_EPOCH_OFFSET: u64 = 2_082_844_800;
/// Files owned by the user running Russet are recorded as this owner, the
/// first user and the `staff` group on a Mac.
const OWNER: (u32, u32) = (501, 20);

/// HFS+ stores names decomposed, except for the ranges TN1150 leaves
/// composed, so macOS finds them by any normalization.
fn hfs_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.nfc() {
        let code = c as u32;
        if (0x2000..=0x2FFF).contains(&code)
            || (0xF900..=0xFAFF).contains(&code)
            || (0x2F800..=0x2FAFF).contains(&code)
        {
            out.push(c);
        } else {
            out.extend(std::iter::once(c).nfd());
        }
    }
    out
}

struct Entry {
    path: String,
    source: std::path::PathBuf,
    metadata: fs::Metadata,
}

fn walk(root: &Path) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut children: Vec<_> = fs::read_dir(&dir)?.collect::<io::Result<_>>()?;
        children.sort_by_key(|e| e.file_name());
        let mut dirs = Vec::new();
        for child in children {
            let name = child.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid(format!("A name in {} isn't UTF-8", dir.display())))?;
            if name.contains(':') {
                return Err(invalid(format!(
                    "{name:?} contains ':', which HFS+ names can't hold"
                )));
            }
            let path = format!("{prefix}/{}", hfs_name(name));
            let metadata = fs::symlink_metadata(child.path())?;
            if metadata.is_dir() {
                dirs.push((child.path(), path.clone()));
            }
            out.push(Entry {
                path,
                source: child.path(),
                metadata,
            });
        }
        // Visit folders in order after their siblings are listed.
        stack.extend(dirs.into_iter().rev());
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn volume_size(entries: &[Entry], journaled: bool) -> u64 {
    let data: u64 = entries
        .iter()
        .map(|e| {
            if e.metadata.is_file() {
                e.metadata.len().div_ceil(BLOCK)
            } else {
                1
            }
        })
        .sum();
    // Generous: a catalog record and thread per entry, at most ~20 per 8 KiB
    // node, plus room for B-tree growth.
    let catalog_nodes = entries.len() as u64 / 8 + 64;
    let journal = if journaled { 8 << 20 } else { 0 };
    let bytes = (data + catalog_nodes * 2 + 64) * BLOCK + journal;
    (bytes + bytes / 10 + (4 << 20)).div_ceil(1 << 20) << 20
}

fn fs_error(e: fstool::Error) -> io::Error {
    invalid(format!("Can't build the HFS+ volume: {e}"))
}

/// Creates a disk image at `image` holding `source`'s contents, like
/// `hdiutil create -srcfolder source -fs FS -format FORMAT image`. The
/// volume is named after `source`. Extended attributes aren't copied, and a
/// journaled file system is written without its journal.
///
/// There's no APFS writer that macOS accepts, so an APFS request writes a
/// case-insensitive HFS+ volume, which macOS reads the same way. Returns
/// the file system written.
pub fn create(source: &Path, image: &Path, options: &CreateOptions) -> io::Result<&'static str> {
    // The journal isn't written: macOS rejects the writer's journal stub, and
    // an image that's only read doesn't need one.
    let journaled = false;
    let written = match options.filesystem {
        "HFS+" | "Journaled HFS+" | "APFS" | "Case-insensitive APFS" => "HFS+",
        other => {
            return Err(invalid(format!(
                "Creating {other} disk images isn't supported natively; use APFS or HFS+"
            )))
        }
    };
    let method = match options.format {
        "UDZO" => udif::CompressionMethod::Zlib,
        "UDBZ" => udif::CompressionMethod::Bzip2,
        "ULFO" => udif::CompressionMethod::Lzfse,
        "UDRO" => udif::CompressionMethod::Raw,
        other => {
            return Err(invalid(format!(
            "Creating {other} disk images isn't supported natively; use UDZO, UDBZ, ULFO, or UDRO"
        )))
        }
    };
    let entries = walk(source)?;
    let size = match options.megabytes {
        Some(megabytes) => megabytes << 20,
        None => volume_size(&entries, journaled),
    };
    let name = source
        .canonicalize()?
        .file_name()
        .and_then(|n| n.to_str())
        .map(hfs_name)
        .unwrap_or_else(|| "untitled".into());
    let scratch = tempfile::tempdir_in(
        image
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let partition = scratch.path().join("volume.hfs");
    {
        let mut device = FileBackend::create(&partition, size).map_err(fs_error)?;
        // The volume dates come from the source folder, keeping the output
        // reproducible. HFS+ counts seconds from 1904.
        let root_mtime = fs::metadata(source)?.mtime().max(0) as u64 + HFS_EPOCH_OFFSET;
        let opts = FormatOpts {
            volume_name: name,
            create_date: u32::try_from(root_mtime).unwrap_or(u32::MAX),
            journaled,
            catalog_nodes: (entries.len() as u32 / 8 + 64).max(32),
            ..Default::default()
        };
        let mut volume = HfsPlus::format(&mut device, &opts).map_err(fs_error)?;
        // The source folder's owner stands for the user running Russet.
        let root = fs::metadata(source)?;
        let (uid, gid) = (root.uid(), root.gid());
        let mut links: HashMap<(u64, u64), String> = HashMap::new();
        for entry in &entries {
            let m = &entry.metadata;
            let mode = (m.permissions().mode() & 0o7777) as u16;
            let owner = (
                if m.uid() == uid { OWNER.0 } else { m.uid() },
                if m.gid() == gid { OWNER.1 } else { m.gid() },
            );
            let mtime = u32::try_from(m.mtime().max(0)).unwrap_or(u32::MAX);
            if m.file_type().is_symlink() {
                let target = fs::read_link(&entry.source)?;
                let target = target
                    .to_str()
                    .ok_or_else(|| invalid("A symlink target isn't UTF-8"))?;
                volume
                    .create_symlink(
                        &mut device,
                        &entry.path,
                        target,
                        mode,
                        owner.0,
                        owner.1,
                        mtime,
                    )
                    .map_err(fs_error)?;
            } else if m.is_dir() {
                volume
                    .create_dir(&mut device, &entry.path, mode, owner.0, owner.1, mtime)
                    .map_err(fs_error)?;
            } else if m.is_file() {
                if m.nlink() > 1 {
                    if let Some(first) = links.get(&(m.dev(), m.ino())) {
                        volume
                            .create_hardlink(&mut device, first, &entry.path)
                            .map_err(fs_error)?;
                        continue;
                    }
                    links.insert((m.dev(), m.ino()), entry.path.clone());
                }
                let mut file = fs::File::open(&entry.source)?.take(m.len());
                volume
                    .create_file(
                        &mut device,
                        &entry.path,
                        &mut file,
                        m.len(),
                        mode,
                        owner.0,
                        owner.1,
                        mtime,
                    )
                    .map_err(fs_error)?;
            }
        }
        volume.flush(&mut device).map_err(fs_error)?;
    }
    let mut data = Vec::with_capacity(size as usize);
    fs::File::open(&partition)?.read_to_end(&mut data)?;
    let staged = scratch.path().join("image.dmg");
    let mut writer = udif::DmgWriter::create(&staged)
        .map_err(|e| invalid(format!("Can't write the disk image: {e}")))?
        .compression(method)
        .compression_level(options.zlib_level);
    writer
        .add_partition("whole disk (Apple_HFS : 0)", &data)
        .map_err(|e| invalid(format!("Can't write the disk image: {e}")))?;
    writer
        .finish()
        .map_err(|e| invalid(format!("Can't write the disk image: {e}")))?;
    fs::rename(&staged, image)?;
    Ok(written)
}
