//! Writes an HFS+ or HFSX volume into a folder, presenting it the way the
//! macOS kernel does when the image is mounted.
//!
//! The walk works from catalog IDs and records only and never looks a name
//! up, because name lookups depend on the HFS+ case-folding rules.

use crate::image::invalid;
use hfsplus::catalog::{CatalogFile, CatalogFolder, CatalogRecord, CNID_ROOT_FOLDER};
use hfsplus::{HfsVolume, XattrKind};
use russet_fs::TreeWriter;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

/// Root folder holding the targets of file hard links.
const FILE_LINKS: &str = "\0\0\0\0HFS+ Private Data";
/// Root folder holding the targets of directory hard links.
const DIRECTORY_LINKS: &str = ".HFS+ Private Directory Data\r";
/// Root entries the kernel hides on a mounted HFS+ volume.
const HIDDEN_ROOT: [&str; 4] = [
    FILE_LINKS,
    DIRECTORY_LINKS,
    ".journal",
    ".journal_info_block",
];
const FILE_HARD_LINK: (&[u8], &[u8]) = (b"hlnk", b"hfs+");
const DIRECTORY_HARD_LINK: (&[u8], &[u8]) = (b"fdrp", b"MACS");
const S_IFMT: u16 = 0o170000;
const S_IFLNK: u16 = 0o120000;
const MAX_LINK_BYTES: u64 = 4096;

struct Walk<'a, R: Read + Seek> {
    volume: HfsVolume<R>,
    writer: &'a mut TreeWriter,
    /// Hard-link targets, by name, in each private folder.
    file_targets: HashMap<String, CatalogFile>,
    directory_targets: HashMap<String, CatalogFolder>,
    /// File hard links already written, by iNode number.
    written: HashMap<u32, PathBuf>,
    /// Directory hard links being expanded, to stop cycles.
    expanding: HashSet<u32>,
}

fn error(e: hfsplus::HfsPlusError) -> io::Error {
    match e {
        hfsplus::HfsPlusError::Io(e) => e,
        other => invalid(format!(
            "Can't read the HFS+ volume in the disk image: {other}"
        )),
    }
}

pub(crate) fn extract<R: Read + Seek>(reader: R, writer: &mut TreeWriter) -> io::Result<()> {
    let mut volume = HfsVolume::open(reader).map_err(error)?;
    let root = volume.children(CNID_ROOT_FOLDER).map_err(error)?;
    let mut file_targets = HashMap::new();
    let mut directory_targets = HashMap::new();
    for (name, record) in &root {
        let CatalogRecord::Folder(folder) = record else {
            continue;
        };
        if name == FILE_LINKS || name == DIRECTORY_LINKS {
            for (child, record) in volume.children(folder.folder_id).map_err(error)? {
                match record {
                    CatalogRecord::File(file) if name == FILE_LINKS => {
                        file_targets.insert(child, file);
                    }
                    CatalogRecord::Folder(folder) if name == DIRECTORY_LINKS => {
                        directory_targets.insert(child, folder);
                    }
                    _ => {}
                }
            }
        }
    }
    let mut walk = Walk {
        volume,
        writer,
        file_targets,
        directory_targets,
        written: HashMap::new(),
        expanding: HashSet::new(),
    };
    let visible = root
        .into_iter()
        .filter(|(name, _)| !HIDDEN_ROOT.contains(&name.as_str()))
        .collect();
    walk.entries(visible, Path::new(""))
}

/// The Finder info that `getxattr` reports for `com.apple.FinderInfo`: the
/// kernel clears the document ID, date added, and write generation counter,
/// and reports nothing when the rest is zero.
fn presented_finder_info(stored: &[u8; 32]) -> Option<Vec<u8>> {
    let mut info = stored.to_vec();
    info[16..24].fill(0);
    info[28..32].fill(0);
    info.iter().any(|b| *b != 0).then_some(info)
}

fn mode(bits: u16) -> u32 {
    u32::from(bits) & 0o7777
}

impl<R: Read + Seek> Walk<'_, R> {
    fn entries(
        &mut self,
        entries: Vec<(String, CatalogRecord)>,
        destination: &Path,
    ) -> io::Result<()> {
        for (name, record) in entries {
            if name.contains('/') {
                return Err(invalid(format!(
                    "A disk image entry named {name:?} can't be represented"
                )));
            }
            let child = destination.join(&name);
            match record {
                CatalogRecord::Folder(folder) => self.folder(&folder, &child)?,
                CatalogRecord::File(file) => self.file(&file, &child)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn folder(&mut self, folder: &CatalogFolder, destination: &Path) -> io::Result<()> {
        self.writer
            .create_dir(destination, Some(mode(folder.permissions.file_mode)))?;
        let children = self.volume.children(folder.folder_id).map_err(error)?;
        self.entries(children, destination)?;
        self.xattrs(folder.folder_id, destination, &folder.finder_info, None)
    }

    fn file(&mut self, file: &CatalogFile, destination: &Path) -> io::Result<()> {
        let info = &file.finder_info;
        let link = file.permissions.special;
        if (&info[0..4], &info[4..8]) == FILE_HARD_LINK {
            if let Some(first) = self.written.get(&link) {
                return self.writer.hard_link(destination, &first.clone());
            }
            let target = self
                .file_targets
                .get(&format!("iNode{link}"))
                .cloned()
                .ok_or_else(|| invalid("A hard link in the disk image has no target"))?;
            self.regular(&target, destination)?;
            self.written.insert(link, destination.to_path_buf());
            return Ok(());
        }
        if (&info[0..4], &info[4..8]) == DIRECTORY_HARD_LINK {
            let target = self
                .directory_targets
                .get(&format!("dir_{link}"))
                .cloned()
                .ok_or_else(|| invalid("A directory hard link in the disk image has no target"))?;
            if !self.expanding.insert(link) {
                return Err(invalid(
                    "Directory hard links in the disk image form a cycle",
                ));
            }
            // Linux can't hard-link directories, so the folder is copied.
            self.folder(&target, destination)?;
            self.expanding.remove(&link);
            return Ok(());
        }
        if file.permissions.file_mode & S_IFMT == S_IFLNK {
            if file.data_fork.logical_size > MAX_LINK_BYTES {
                return Err(invalid("A symlink in the disk image is too long"));
            }
            let mut target = Vec::new();
            self.volume
                .read_record_to(file, &mut target)
                .map_err(error)?;
            let target = std::ffi::OsString::from(String::from_utf8_lossy(&target).into_owned());
            return self.writer.symlink(destination, &target);
        }
        self.regular(file, destination)
    }

    fn regular(&mut self, file: &CatalogFile, destination: &Path) -> io::Result<()> {
        let volume = &mut self.volume;
        let mut failure = None;
        self.writer
            .write_file_with(destination, mode(file.permissions.file_mode), |sink| {
                volume.read_record_to(file, sink).map(|_| ()).map_err(|e| {
                    let e = error(e);
                    failure = Some(e.to_string());
                    e
                })
            })
            .map_err(|e| failure.map(invalid).unwrap_or(e))?;
        // A compressed file keeps its payload in the resource fork, which the
        // kernel doesn't present.
        let compressed = self.volume.is_compressed(file.file_id).map_err(error)?;
        let fork = (!compressed && file.resource_fork.logical_size > 0).then_some(file);
        self.xattrs(file.file_id, destination, &file.finder_info, fork)
    }

    fn xattrs(
        &mut self,
        id: u32,
        destination: &Path,
        finder_info: &[u8; 32],
        resource_fork: Option<&CatalogFile>,
    ) -> io::Result<()> {
        if let Some(info) = presented_finder_info(finder_info) {
            self.writer
                .set_xattr(destination, "com.apple.FinderInfo", &info)?;
        }
        if let Some(file) = resource_fork {
            let fork = self.volume.read_record_resource_fork(file).map_err(error)?;
            self.writer
                .set_xattr(destination, "com.apple.ResourceFork", &fork)?;
        }
        for attribute in self.volume.list_xattrs_by_id(id).map_err(error)? {
            if attribute.kind != XattrKind::User {
                continue;
            }
            if let Some(value) = self
                .volume
                .get_xattr_by_id(id, &attribute.name)
                .map_err(error)?
            {
                self.writer
                    .set_xattr(destination, &attribute.name, &value)?;
            }
        }
        Ok(())
    }
}
