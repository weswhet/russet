//! Writes portable (`odc`) cpio archives, the format of macOS package
//! payloads and Scripts archives, with ownership supplied by the caller.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The largest file size an `odc` header can record (11 octal digits).
const MAX_SIZE: u64 = 0o77777777777;

/// Header values for one cpio member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// File type and permission bits, such as `0o100644`.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u64,
    /// Members with the same `ino` and `nlink` above 1 are hard links.
    pub ino: u64,
    pub nlink: u32,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Streams `odc` members to a writer.
pub struct CpioWriter<W: Write> {
    out: W,
}

impl<W: Write> CpioWriter<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// Appends a member. `data` must yield exactly `size` bytes.
    pub fn append(
        &mut self,
        name: &str,
        header: Header,
        data: impl Read,
        size: u64,
    ) -> io::Result<()> {
        if size > MAX_SIZE {
            return Err(invalid(format!("{name} is too large for a cpio archive")));
        }
        let field = |value: u64, width: u32, what: &str| -> io::Result<String> {
            if value >= 8u64.pow(width) {
                return Err(invalid(format!(
                    "{what} of {name} doesn't fit in a cpio header"
                )));
            }
            Ok(format!("{value:0width$o}", width = width as usize))
        };
        let text = format!(
            "070707{}{}{}{}{}{}{}{}{}{}",
            field(0, 6, "device")?,
            field(header.ino % 0o1000000, 6, "inode")?,
            field(u64::from(header.mode), 6, "mode")?,
            field(u64::from(header.uid), 6, "owner")?,
            field(u64::from(header.gid), 6, "group")?,
            field(u64::from(header.nlink), 6, "link count")?,
            field(0, 6, "rdev")?,
            field(header.mtime, 11, "modification time")?,
            field(name.len() as u64 + 1, 6, "name length")?,
            field(size, 11, "size")?,
        );
        self.out.write_all(text.as_bytes())?;
        self.out.write_all(name.as_bytes())?;
        self.out.write_all(&[0])?;
        let copied = io::copy(&mut data.take(size), &mut self.out)?;
        if copied != size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{name} changed size while it was archived"),
            ));
        }
        Ok(())
    }

    /// Writes the trailer and returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        let header = Header {
            mode: 0,
            uid: 0,
            gid: 0,
            mtime: 0,
            ino: 0,
            nlink: 1,
        };
        self.append("TRAILER!!!", header, io::empty(), 0)?;
        Ok(self.out)
    }
}

/// Archives the tree at `root` as `.` and `./relative/path` members, in
/// sorted order, the way `ditto -c` does. `header` receives each path
/// relative to `root` (empty for the root) and its `lstat` metadata, and
/// returns the header to record, so callers can supply ownership and modes
/// that differ from the disk. Hard links are recorded with their data
/// repeated, as `ditto` writes them.
pub fn write_tree<W: Write>(
    root: &Path,
    out: W,
    mut header: impl FnMut(&Path, &fs::Metadata) -> Header,
) -> io::Result<W> {
    let mut writer = CpioWriter::new(out);
    let mut inodes: HashMap<(u64, u64), u64> = HashMap::new();
    let mut walk = vec![std::path::PathBuf::new()];
    while let Some(relative) = walk.pop() {
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        let name = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            format!(
                "./{}",
                relative
                    .to_str()
                    .ok_or_else(|| invalid(format!("{} isn't UTF-8", relative.display())))?
            )
        };
        let next = inodes.len() as u64 + 1;
        let ino = *inodes
            .entry((metadata.dev(), metadata.ino()))
            .or_insert(next);
        let mut values = header(&relative, &metadata);
        values.ino = ino;
        let kind = metadata.file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            let target = target.as_os_str().as_encoded_bytes();
            writer.append(&name, values, target, target.len() as u64)?;
        } else if kind.is_dir() {
            writer.append(&name, values, io::empty(), 0)?;
            let mut children: Vec<_> = fs::read_dir(&path)?
                .map(|e| e.map(|e| relative.join(e.file_name())))
                .collect::<io::Result<_>>()?;
            // Pushed in reverse so the walk visits them in sorted order.
            children.sort_by(|a, b| b.cmp(a));
            walk.extend(children);
        } else if kind.is_file() {
            writer.append(&name, values, fs::File::open(&path)?, metadata.len())?;
        }
    }
    writer.finish()
}
