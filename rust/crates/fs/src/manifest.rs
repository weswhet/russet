use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

/// Attributes the host adds on its own, which never describe archive content.
const HOST_XATTRS: [&str; 2] = ["com.apple.provenance", "com.apple.quarantine"];

/// The kind of a tree entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Directory,
    File,
    Symlink,
}

/// One entry of a [`manifest`]. File contents and attribute values are
/// recorded as SHA-256 digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub mode: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub xattrs: BTreeMap<String, String>,
    /// Entries that share an inode get the same group number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_group: Option<usize>,
}

/// Describes every entry below `root` (not `root` itself), sorted by path.
///
/// Extended attributes use their Apple names, so a manifest taken on Linux
/// compares directly with one taken on macOS. Attributes the host adds on its
/// own, such as `com.apple.provenance`, are left out.
pub fn manifest(root: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut inodes = BTreeMap::<(u64, u64), Vec<usize>>::new();
    walk(root, root, &mut entries, &mut inodes)?;
    let mut groups: Vec<Vec<usize>> = inodes.into_values().filter(|v| v.len() > 1).collect();
    groups.sort();
    for (group, members) in groups.into_iter().enumerate() {
        for index in members {
            entries[index].link_group = Some(group);
        }
    }
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by(|a, b| entries[*a].path.as_bytes().cmp(entries[*b].path.as_bytes()));
    let sorted = order.into_iter().map(|i| entries[i].clone()).collect();
    Ok(sorted)
}

fn walk(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<Entry>,
    inodes: &mut BTreeMap<(u64, u64), Vec<usize>>,
) -> io::Result<()> {
    let mut children: Vec<_> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
    children.sort_by_key(|c| c.file_name());
    for child in children {
        // Attributes the host refused are folded into their files' entries.
        if dir == root && child.file_name() == crate::SIDECAR {
            continue;
        }
        let path = child.path();
        let meta = fs::symlink_metadata(&path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace('\\', "/");
        let kind = if meta.file_type().is_symlink() {
            EntryKind::Symlink
        } else if meta.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        let mut entry = Entry {
            path: relative,
            kind,
            mode: meta.permissions().mode() & 0o7777,
            size: None,
            sha256: None,
            target: None,
            xattrs: xattrs(&path)?,
            link_group: None,
        };
        match kind {
            EntryKind::File => {
                entry.size = Some(meta.len());
                entry.sha256 = Some(hex(&Sha256::digest(fs::read(&path)?)));
                inodes
                    .entry((meta.dev(), meta.ino()))
                    .or_default()
                    .push(entries.len());
            }
            EntryKind::Symlink => {
                entry.target = Some(fs::read_link(&path)?.to_string_lossy().into_owned());
                // Symlink permission bits are meaningless on Linux and vary on
                // macOS with the umask, so they aren't compared.
                entry.mode = 0;
            }
            EntryKind::Directory => {}
        }
        entries.push(entry);
        if kind == EntryKind::Directory {
            walk(root, &path, entries, inodes)?;
        }
    }
    Ok(())
}

fn xattrs(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    let names = match crate::list_xattrs(path) {
        Ok(names) => names,
        Err(e) if e.kind() == io::ErrorKind::Unsupported => return Ok(values),
        Err(e) => return Err(e),
    };
    for name in names {
        // Linux's own namespaces, such as security.selinux, aren't Apple's.
        let host_only = cfg!(target_os = "linux")
            && ["security.", "trusted.", "system."]
                .iter()
                .any(|ns| name.starts_with(ns));
        if HOST_XATTRS.contains(&name.as_str()) || host_only {
            continue;
        }
        if let Some(value) = crate::get_xattr(path, &name)? {
            values.insert(name, hex(&Sha256::digest(value)));
        }
    }
    Ok(values)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
