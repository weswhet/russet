use crate::sidecar::{encode, SIDECAR};
use crate::{clean_relative, host_xattr_name, invalid, Limits};
use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags, XattrFlags};
use rustix::io::Errno;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

const DIR_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

/// An extended attribute the host filesystem couldn't store.
///
/// Linux refuses `user.` attributes on symlinks, and many filesystems cap
/// attribute size below what a resource fork can need. Callers decide whether
/// a skipped attribute matters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedXattr {
    /// Path relative to the destination.
    pub path: PathBuf,
    /// Apple attribute name, such as `com.apple.ResourceFork`.
    pub name: String,
    /// Why the host refused it.
    pub reason: String,
}

/// Writes an untrusted tree below one destination directory.
///
/// Every path is cleaned with [`clean_relative`], and each directory on the
/// way is opened relative to its parent with `O_NOFOLLOW`, so a symlink in the
/// destination or in the archive can't redirect a write. Files are created
/// with `O_EXCL` after any existing non-directory entry is removed. Symlinks
/// are created and directory modes are applied in [`TreeWriter::finish`], so
/// read-only directories and links never block later entries.
pub struct TreeWriter {
    root: OwnedFd,
    limits: Limits,
    written: u64,
    entries: u64,
    symlinks: Vec<(PathBuf, OsString)>,
    dir_modes: Vec<(PathBuf, u32)>,
    skipped: Vec<SkippedXattr>,
}

impl TreeWriter {
    /// Opens `root`, which must be an existing directory.
    pub fn open(root: &Path, limits: Limits) -> io::Result<Self> {
        // Attributes kept by an earlier extraction into this folder are
        // stale. remove_dir_all doesn't follow symlinks.
        let stale = root.join(SIDECAR);
        match std::fs::symlink_metadata(&stale) {
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&stale)?,
            Ok(_) => std::fs::remove_file(&stale)?,
            Err(_) => {}
        }
        let root = rfs::open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            root,
            limits,
            written: 0,
            entries: 0,
            symlinks: Vec::new(),
            dir_modes: Vec::new(),
            skipped: Vec::new(),
        })
    }

    /// Keeps an attribute the host refused in the sidecar folder (see
    /// [`crate::get_xattr`]).
    fn keep_in_sidecar(&mut self, rel: &Path, name: &str, value: &[u8]) -> io::Result<()> {
        let dir = self.open_dir(&Path::new(SIDECAR).join(rel), true)?;
        let file_name = encode(name);
        let _ = rfs::unlinkat(&dir, file_name.as_str(), AtFlags::empty());
        let fd = rfs::openat(
            &dir,
            file_name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o644),
        )?;
        File::from(fd).write_all(value)?;
        self.written += value.len() as u64;
        Ok(())
    }

    /// Bytes written so far to files and extended attributes.
    pub fn bytes_written(&self) -> u64 {
        self.written
    }

    /// Creates a directory and any missing parents. `mode` is applied when
    /// the writer finishes; missing parents get `0755`.
    pub fn create_dir(&mut self, path: &Path, mode: Option<u32>) -> io::Result<()> {
        let rel = self.admit(path)?;
        if rel.as_os_str().is_empty() {
            return Ok(());
        }
        self.open_dir(&rel, true)?;
        if let Some(mode) = mode {
            self.dir_modes.push((rel, mode & 0o7777));
        }
        Ok(())
    }

    /// Writes a regular file from `reader`, replacing any existing file or
    /// symlink at that path. Set-user-ID and set-group-ID bits are dropped.
    pub fn write_file(&mut self, path: &Path, reader: impl Read, mode: u32) -> io::Result<u64> {
        let rel = self.admit(path)?;
        let (parent, name) = split(&rel)?;
        let dir = self.open_dir(&parent, true)?;
        remove_non_directory(&dir, &name, &rel)?;
        let fd = rfs::openat(
            &dir,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            perm(0o600),
        )?;
        let mut file = File::from(fd);
        let remaining = self.limits.max_total_bytes - self.written;
        let copied = io::copy(&mut reader.take(remaining.saturating_add(1)), &mut file)?;
        if copied > remaining {
            return Err(invalid("Extraction exceeds the total size limit"));
        }
        self.written += copied;
        rfs::fchmod(&file, perm(mode & 0o1777))?;
        Ok(copied)
    }

    /// Writes a regular file whose contents a callback produces by writing to
    /// the provided sink, for sources that write rather than read. The total
    /// size limit applies as it does for [`TreeWriter::write_file`].
    pub fn write_file_with(
        &mut self,
        path: &Path,
        mode: u32,
        produce: impl FnOnce(&mut dyn Write) -> io::Result<()>,
    ) -> io::Result<u64> {
        let rel = self.admit(path)?;
        let (parent, name) = split(&rel)?;
        let dir = self.open_dir(&parent, true)?;
        remove_non_directory(&dir, &name, &rel)?;
        let fd = rfs::openat(
            &dir,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            perm(0o600),
        )?;
        let mut sink = Limited {
            file: File::from(fd),
            remaining: self.limits.max_total_bytes - self.written,
            written: 0,
        };
        produce(&mut sink)?;
        self.written += sink.written;
        rfs::fchmod(&sink.file, perm(mode & 0o1777))?;
        Ok(sink.written)
    }

    /// Queues a symlink. Links are created last and are never followed.
    pub fn symlink(&mut self, path: &Path, target: &OsStr) -> io::Result<()> {
        let rel = self.admit(path)?;
        if target.is_empty() || target.as_encoded_bytes().contains(&0) {
            return Err(invalid(format!(
                "Archive contains an invalid symlink target for '{}'",
                path.display()
            )));
        }
        split(&rel)?;
        self.symlinks.push((rel, target.to_owned()));
        Ok(())
    }

    /// Creates `path` as a hard link to the regular file already written at
    /// `existing`.
    pub fn hard_link(&mut self, path: &Path, existing: &Path) -> io::Result<()> {
        let rel = self.admit(path)?;
        let source = clean_relative(existing)?;
        let (source_parent, source_name) = split(&source)?;
        let (parent, name) = split(&rel)?;
        let source_dir = self.open_dir(&source_parent, false)?;
        let stat = rfs::statat(&source_dir, &source_name, AtFlags::SYMLINK_NOFOLLOW)?;
        if FileType::from_raw_mode(stat.st_mode as _) != FileType::RegularFile {
            return Err(invalid(format!(
                "Hard link '{}' doesn't name a regular file",
                rel.display()
            )));
        }
        let dir = self.open_dir(&parent, true)?;
        remove_non_directory(&dir, &name, &rel)?;
        rfs::linkat(&source_dir, &source_name, &dir, &name, AtFlags::empty())?;
        Ok(())
    }

    /// Sets an extended attribute, using the Apple name (see
    /// [`host_xattr_name`]). Attributes the host refuses are recorded and
    /// reported by [`TreeWriter::finish`] instead of failing the extraction.
    pub fn set_xattr(&mut self, path: &Path, name: &str, value: &[u8]) -> io::Result<()> {
        let rel = clean_relative(path)?;
        if rel
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == SIDECAR)
        {
            return Err(invalid(format!("'{}' is reserved", rel.display())));
        }
        let remaining = self.limits.max_total_bytes - self.written;
        if value.len() as u64 > remaining {
            return Err(invalid("Extraction exceeds the total size limit"));
        }
        let skip = |reason: String| SkippedXattr {
            path: rel.clone(),
            name: name.to_owned(),
            reason,
        };
        if self.symlinks.iter().any(|(link, _)| *link == rel) {
            // Symlinks are created last, and Linux refuses their attributes.
            if let Err(e) = self.keep_in_sidecar(&rel, name, value) {
                self.skipped.push(skip(format!("symlink: {e}")));
            }
            return Ok(());
        }
        let fd = if rel.as_os_str().is_empty() {
            rfs::openat(&self.root, ".", DIR_FLAGS, Mode::empty())?
        } else {
            let (parent, entry) = split(&rel)?;
            let dir = self.open_dir(&parent, false)?;
            let stat = rfs::statat(&dir, &entry, AtFlags::SYMLINK_NOFOLLOW)?;
            match FileType::from_raw_mode(stat.st_mode as _) {
                FileType::Directory => rfs::openat(&dir, &entry, DIR_FLAGS, Mode::empty())?,
                FileType::RegularFile => open_for_xattr(&dir, &entry, stat.st_mode as u32)?,
                _ => {
                    self.skipped.push(skip("not a file or directory".into()));
                    return Ok(());
                }
            }
        };
        match rfs::fsetxattr(
            &fd,
            host_xattr_name(name).as_ref(),
            value,
            XattrFlags::empty(),
        ) {
            Ok(()) => {
                self.written += value.len() as u64;
                Ok(())
            }
            Err(
                errno @ (Errno::NOTSUP | Errno::TOOBIG | Errno::NOSPC | Errno::PERM | Errno::RANGE),
            ) => {
                if let Err(e) = self.keep_in_sidecar(&rel, name, value) {
                    self.skipped
                        .push(skip(format!("{}; sidecar: {e}", io::Error::from(errno))));
                }
                Ok(())
            }
            Err(errno) => Err(errno.into()),
        }
    }

    /// Creates queued symlinks, applies directory modes deepest first, and
    /// returns the attributes the host couldn't store.
    pub fn finish(self) -> io::Result<Vec<SkippedXattr>> {
        for (rel, target) in &self.symlinks {
            let (parent, name) = split(rel)?;
            let dir = self.open_dir(&parent, true)?;
            remove_non_directory(&dir, &name, rel)?;
            rfs::symlinkat(target.as_os_str(), &dir, &name)?;
        }
        let mut modes = self.dir_modes.clone();
        modes.sort_by_key(|(rel, _)| std::cmp::Reverse(rel.components().count()));
        for (rel, mode) in modes {
            let fd = self.open_dir(&rel, false)?;
            rfs::fchmod(&fd, perm(mode))?;
        }
        Ok(self.skipped)
    }

    fn admit(&mut self, path: &Path) -> io::Result<PathBuf> {
        let rel = clean_relative(path)?;
        if rel
            .components()
            .next()
            .is_some_and(|c| c.as_os_str() == SIDECAR)
        {
            return Err(invalid(format!(
                "Archive path '{}' uses the reserved name {SIDECAR}",
                rel.display()
            )));
        }
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(invalid("Extraction exceeds the entry limit"));
        }
        if rel.components().count() > self.limits.max_depth {
            return Err(invalid(format!(
                "Archive path '{}' exceeds the depth limit",
                rel.display()
            )));
        }
        if rel
            .components()
            .any(|c| c.as_os_str().len() > self.limits.max_name_bytes)
        {
            return Err(invalid(format!(
                "Archive path '{}' has a name longer than {} bytes",
                rel.display(),
                self.limits.max_name_bytes
            )));
        }
        Ok(rel)
    }

    /// Opens the directory at `rel` without following symlinks, creating
    /// missing components when `create` is set.
    fn open_dir(&self, rel: &Path, create: bool) -> io::Result<OwnedFd> {
        let mut fd = rfs::openat(&self.root, ".", DIR_FLAGS, Mode::empty())?;
        for part in rel.components() {
            let name = part.as_os_str();
            fd = match rfs::openat(&fd, name, DIR_FLAGS, Mode::empty()) {
                Ok(next) => next,
                Err(Errno::NOENT) if create => {
                    match rfs::mkdirat(&fd, name, perm(0o755)) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(errno) => return Err(errno.into()),
                    }
                    rfs::openat(&fd, name, DIR_FLAGS, Mode::empty())?
                }
                Err(Errno::LOOP | Errno::NOTDIR | Errno::MLINK) => {
                    return Err(invalid(format!(
                        "Archive path '{}' passes through a symlink or file",
                        rel.display()
                    )))
                }
                Err(errno) => return Err(errno.into()),
            };
        }
        Ok(fd)
    }
}

/// Converts permission bits; the raw mode type differs between Linux and macOS.
fn perm(bits: u32) -> Mode {
    Mode::from_raw_mode((bits & 0o7777) as rfs::RawMode)
}

fn split(rel: &Path) -> io::Result<(PathBuf, OsString)> {
    match (rel.parent(), rel.file_name()) {
        (Some(parent), Some(name)) => Ok((parent.to_path_buf(), name.to_owned())),
        _ => Err(invalid("Archive entry names the destination itself")),
    }
}

fn remove_non_directory(dir: &OwnedFd, name: &OsStr, rel: &Path) -> io::Result<()> {
    match rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode as _) == FileType::Directory => {
            Err(invalid(format!(
                "Archive entry '{}' would replace a directory",
                rel.display()
            )))
        }
        Ok(_) => Ok(rfs::unlinkat(dir, name, AtFlags::empty())?),
        Err(Errno::NOENT) => Ok(()),
        Err(errno) => Err(errno.into()),
    }
}

/// Opens a regular file to change its attributes. A file whose mode denies
/// the owner read access is opened after briefly granting it, then restored.
fn open_for_xattr(dir: &OwnedFd, name: &OsStr, mode: u32) -> io::Result<OwnedFd> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match rfs::openat(dir, name, flags, Mode::empty()) {
        Err(Errno::ACCESS) => {
            let original = perm(mode & 0o7777);
            let fd = rfs::openat(
                dir,
                name,
                OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .or_else(|_| {
                rfs::chmodat(dir, name, original | Mode::RUSR, AtFlags::empty())?;
                rfs::openat(dir, name, flags, Mode::empty())
            })?;
            rfs::fchmod(&fd, original)?;
            Ok(fd)
        }
        other => Ok(other?),
    }
}

/// A file writer that stops at the extraction's size limit.
struct Limited {
    file: File,
    remaining: u64,
    written: u64,
}

impl Write for Limited {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.remaining - self.written {
            return Err(invalid("Extraction exceeds the total size limit"));
        }
        let count = self.file.write(buf)?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
