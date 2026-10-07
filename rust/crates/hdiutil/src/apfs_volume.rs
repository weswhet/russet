//! Writes the first volume of an APFS container into a folder, presenting it
//! the way the macOS kernel does when the image is mounted.

use crate::image::invalid;
use apfs::{ApfsVolume, EntryKind, XattrKind};
use russet_fs::TreeWriter;
use std::collections::HashMap;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};

/// Attributes APFS uses internally; the kernel doesn't present them.
const INTERNAL: [&str; 1] = ["com.apple.fs.symlink"];

fn error(path: &str, e: apfs::ApfsError) -> io::Error {
    match e {
        apfs::ApfsError::Io(e) => e,
        other => invalid(format!("Can't read {path:?} in the disk image: {other}")),
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{parent}/{name}")
    }
}

struct Walk<'a, R: Read + Seek> {
    volume: ApfsVolume<R>,
    writer: &'a mut TreeWriter,
    /// Files with more than one link already written, by object ID.
    inodes: HashMap<u64, PathBuf>,
}

pub(crate) fn extract<R: Read + Seek>(reader: R, writer: &mut TreeWriter) -> io::Result<()> {
    let volume = ApfsVolume::open(reader).map_err(|e| error("/", e))?;
    let mut walk = Walk {
        volume,
        writer,
        inodes: HashMap::new(),
    };
    walk.directory("/", Path::new(""))
}

impl<R: Read + Seek> Walk<'_, R> {
    fn directory(&mut self, source: &str, destination: &Path) -> io::Result<()> {
        let entries = self
            .volume
            .list_directory(source)
            .map_err(|e| error(source, e))?;
        for entry in entries {
            if entry.name.contains('/') {
                return Err(invalid(format!(
                    "{:?} in the disk image has a name Russet can't represent",
                    join(source, &entry.name)
                )));
            }
            let child_source = join(source, &entry.name);
            let child = destination.join(&entry.name);
            let stat = self
                .volume
                .stat(&child_source)
                .map_err(|e| error(&child_source, e))?;
            let mode = u32::from(stat.mode) & 0o7777;
            match entry.kind {
                EntryKind::Directory => {
                    self.writer.create_dir(&child, Some(mode))?;
                    self.directory(&child_source, &child)?;
                }
                EntryKind::Symlink => {
                    let target = self
                        .volume
                        .read_file(&child_source)
                        .map_err(|e| error(&child_source, e))?;
                    let target = String::from_utf8_lossy(&target);
                    let target = target.trim_end_matches('\0');
                    self.writer.symlink(&child, std::ffi::OsStr::new(target))?;
                    continue;
                }
                EntryKind::File => {
                    if stat.nlink > 1 {
                        if let Some(first) = self.inodes.get(&stat.oid) {
                            self.writer.hard_link(&child, &first.clone())?;
                            continue;
                        }
                        self.inodes.insert(stat.oid, child.clone());
                    }
                    if stat.compression.is_some() {
                        let data = self
                            .volume
                            .read_file(&child_source)
                            .map_err(|e| error(&child_source, e))?;
                        self.writer.write_file(&child, data.as_slice(), mode)?;
                    } else {
                        let reader = self
                            .volume
                            .open_file(&child_source)
                            .map_err(|e| error(&child_source, e))?;
                        let written =
                            self.writer
                                .write_file(&child, reader.take(stat.size), mode)?;
                        if written != stat.size {
                            return Err(invalid(format!(
                                "{child_source:?} in the disk image ends early"
                            )));
                        }
                    }
                }
            }
            self.xattrs(&child_source, &child)?;
        }
        Ok(())
    }

    fn xattrs(&mut self, source: &str, destination: &Path) -> io::Result<()> {
        for attribute in self
            .volume
            .list_xattrs(source)
            .map_err(|e| error(source, e))?
        {
            if attribute.kind != XattrKind::User || INTERNAL.contains(&attribute.name.as_str()) {
                continue;
            }
            if let Some(value) = self
                .volume
                .get_xattr(source, &attribute.name)
                .map_err(|e| error(source, e))?
            {
                self.writer
                    .set_xattr(destination, &attribute.name, &value)?;
            }
        }
        Ok(())
    }
}
