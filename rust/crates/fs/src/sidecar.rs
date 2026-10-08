//! Extended attributes the host filesystem refuses.
//!
//! Linux filesystems such as ext4 limit the size of all of a file's extended
//! attributes together (about 4 KiB), and Linux refuses `user.` attributes on
//! symlinks. Apple software stores larger ones: resource forks, and the
//! `com.apple.cs.*` signatures of non-Mach-O files in a bundle. [`TreeWriter`]
//! keeps an attribute the host refuses in a sidecar folder at the top of the
//! extraction, `.russet-xattrs/<path>/<name>`, and [`get_xattr`] and
//! [`list_xattrs`] read both places, so a code signature check sees what
//! macOS would.
//!
//! [`TreeWriter`]: crate::TreeWriter

use crate::{apple_xattr_name, host_xattr_name};
use std::io;
use std::path::{Path, PathBuf};

/// The sidecar folder's name at the top of an extraction.
pub const SIDECAR: &str = ".russet-xattrs";

/// Encodes an attribute name as one file name.
pub(crate) fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (index, c) in name.chars().enumerate() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            '\0' => out.push_str("%00"),
            '.' if index == 0 => out.push_str("%2E"),
            c => out.push(c),
        }
    }
    out
}

fn decode(name: &str) -> String {
    name.replace("%2F", "/")
        .replace("%00", "\0")
        .replace("%2E", ".")
        .replace("%25", "%")
}

/// Whether `path` is a folder, without following a symlink.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// The sidecar folder holding `path`'s refused attributes, if any extraction
/// above it kept some. Symlinks are never followed, so an extracted tree
/// can't point the sidecar at other files.
fn sidecar_for(path: &Path) -> Option<PathBuf> {
    'ancestors: for ancestor in path.ancestors().skip(1) {
        let sidecar = ancestor.join(SIDECAR);
        if !real_dir(&sidecar) {
            continue;
        }
        let relative = path.strip_prefix(ancestor).ok()?;
        let mut folder = sidecar;
        for part in relative.components() {
            folder.push(part);
            if !real_dir(&folder) {
                continue 'ancestors;
            }
        }
        return Some(folder);
    }
    None
}

/// Reads extended attribute `name` (an Apple name, such as
/// `com.apple.ResourceFork`) of `path`, without following a symlink, from
/// the file or its extraction's sidecar.
pub fn get_xattr(path: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    match xattr::get(path, host_xattr_name(name).as_ref()) {
        Ok(Some(value)) => return Ok(Some(value)),
        Ok(None) => {}
        // Linux refuses `user.` attributes on symlinks.
        Err(e) if e.raw_os_error() == Some(1) => {}
        Err(e) => return Err(e),
    }
    let Some(folder) = sidecar_for(path) else {
        return Ok(None);
    };
    let file = folder.join(encode(name));
    match std::fs::symlink_metadata(&file) {
        Ok(metadata) if metadata.is_file() => std::fs::read(&file).map(Some),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The Apple names of `path`'s extended attributes, from the file and its
/// extraction's sidecar.
pub fn list_xattrs(path: &Path) -> io::Result<Vec<String>> {
    let mut names: Vec<String> = match xattr::list(path) {
        Ok(names) => names
            .filter_map(|n| n.to_str().map(|n| apple_xattr_name(n).to_owned()))
            .collect(),
        Err(e) if e.raw_os_error() == Some(1) => Vec::new(),
        Err(e) => return Err(e),
    };
    if let Some(folder) = sidecar_for(path) {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(decode(name));
                }
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use crate::{get_xattr, list_xattrs, manifest, Limits, TreeWriter, SIDECAR};
    use std::path::Path;

    /// Attributes the host refuses — on a symlink, or too large for ext4 —
    /// are kept in the sidecar and read back as if stored on the file.
    #[test]
    fn keeps_refused_attributes_in_the_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let large = vec![7u8; 64 << 10];
        let mut writer = TreeWriter::open(root, Limits::default()).unwrap();
        writer
            .create_dir(Path::new("App.app"), Some(0o755))
            .unwrap();
        writer
            .write_file(Path::new("App.app/file"), &b"data"[..], 0o644)
            .unwrap();
        writer
            .set_xattr(
                Path::new("App.app/file"),
                "com.apple.cs.CodeSignature",
                &large,
            )
            .unwrap();
        writer
            .symlink(Path::new("App.app/link"), "file".as_ref())
            .unwrap();
        writer
            .set_xattr(Path::new("App.app/link"), "com.apple.FinderInfo", b"info")
            .unwrap();
        assert!(writer.finish().unwrap().is_empty());

        let file = root.join("App.app/file");
        let link = root.join("App.app/link");
        assert_eq!(
            get_xattr(&file, "com.apple.cs.CodeSignature").unwrap(),
            Some(large)
        );
        assert_eq!(
            get_xattr(&link, "com.apple.FinderInfo").unwrap().as_deref(),
            Some(&b"info"[..])
        );
        assert_eq!(get_xattr(&file, "com.apple.missing").unwrap(), None);
        assert!(list_xattrs(&link)
            .unwrap()
            .contains(&"com.apple.FinderInfo".to_owned()));
        // The sidecar isn't part of the tree, but its attributes are.
        let entries = manifest(root).unwrap();
        assert!(entries.iter().all(|e| !e.path.starts_with(SIDECAR)));
        let link_entry = entries.iter().find(|e| e.path == "App.app/link").unwrap();
        assert!(link_entry.xattrs.contains_key("com.apple.FinderInfo"));

        // Archives can't write into the sidecar, and a new extraction into
        // the same folder drops stale attributes.
        let mut writer = TreeWriter::open(root, Limits::default()).unwrap();
        assert!(writer
            .write_file(&Path::new(SIDECAR).join("x"), &b""[..], 0o644)
            .is_err());
        // The name is reserved at every depth, so an archive can't point a
        // nested sidecar elsewhere.
        assert!(writer
            .symlink(&Path::new("App.app").join(SIDECAR), "/".as_ref())
            .is_err());
        assert!(!root.join(SIDECAR).exists());
        assert_eq!(get_xattr(&link, "com.apple.FinderInfo").unwrap(), None);
    }

    /// A sidecar, or a folder in it, that is a symlink is ignored.
    #[test]
    fn never_follows_symlinks_into_or_inside_the_sidecar() {
        let victim = tempfile::tempdir().unwrap();
        std::fs::create_dir(victim.path().join("file")).unwrap();
        std::fs::write(victim.path().join("file/com.example"), b"host").unwrap();
        std::fs::write(victim.path().join("com.example"), b"host").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("file"), b"data").unwrap();
        std::os::unix::fs::symlink(victim.path(), root.join(SIDECAR)).unwrap();
        assert_eq!(get_xattr(&root.join("file"), "com.example").unwrap(), None);
        assert!(!list_xattrs(&root.join("file"))
            .unwrap()
            .contains(&"com.example".to_owned()));

        std::fs::remove_file(root.join(SIDECAR)).unwrap();
        std::fs::create_dir(root.join(SIDECAR)).unwrap();
        std::os::unix::fs::symlink(victim.path().join("file"), root.join(SIDECAR).join("file"))
            .unwrap();
        assert_eq!(get_xattr(&root.join("file"), "com.example").unwrap(), None);
    }
}
