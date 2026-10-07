use crate::common::{command, lchown, mountpoint, numeric_id, realpath, remove, string, walk};
use plist::{Dictionary, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::Command,
};

fn no_parent(path: &str, field: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err(format!("{field} is required"));
    }
    if path.replace('\\', "/").split('/').any(|p| p == "..") {
        return Err(format!(
            "{field} may not contain parent-directory references"
        ));
    }
    Ok(())
}
fn relative(path: &str, field: &str) -> Result<(), String> {
    no_parent(path, field)?;
    if Path::new(path).is_absolute() {
        return Err(format!("{field} must be relative"));
    }
    Ok(())
}
fn mode(item: &Dictionary) -> Result<&str, String> {
    item.get("mode")
        .map(|v| v.as_string().ok_or("mode must be a string".into()))
        .unwrap_or(Ok("o-w"))
}
fn verify_mode(mode: &str) -> Result<(), String> {
    let value = mode.trim();
    if value.is_empty() {
        return Err("mode is required".into());
    }
    if value.bytes().all(|b| matches!(b, b'0'..=b'7')) {
        let numeric = u32::from_str_radix(value, 8).map_err(|_| "Invalid numeric mode")?;
        if numeric & 0o6000 != 0 {
            return Err("mode may not set setuid or setgid bits".into());
        }
    } else if regex::Regex::new(r"(^|,)[ugoa]*[+=][^,]*s")
        .unwrap()
        .is_match(&value.to_lowercase())
    {
        return Err("mode may not set setuid or setgid bits".into());
    }
    Ok(())
}
fn verify_mount(
    request: &Dictionary,
    temporary: &Path,
    mounted: impl Fn(&Path) -> bool,
) -> Result<PathBuf, String> {
    let path = string(request, "mount_point")?;
    no_parent(path, "mount_point")?;
    let resolved = realpath(Path::new(path))?;
    if !resolved.starts_with(temporary) {
        return Err("mount_point is not in an allowed location".into());
    }
    if !resolved.is_dir() {
        return Err(format!("mount_point {path} is not a directory"));
    }
    if !mounted(&resolved) {
        return Err("mount_point is not a mounted volume".into());
    }
    Ok(resolved)
}
fn paths(item: &Dictionary, mount: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let source = string(item, "source_item").map_err(|_| "source_item is required")?;
    relative(source, "source_item")?;
    let source_path = mount.join(source);
    if !realpath(&source_path)?.starts_with(mount) {
        return Err(format!(
            "Source item {source} resolves outside the mount point"
        ));
    }
    if !source_path.exists() {
        return Err(format!("Source item {source} does not exist!"));
    }
    let destination =
        string(item, "destination_path").map_err(|_| "destination_path is required")?;
    no_parent(destination, "destination_path")?;
    if !Path::new(destination).is_absolute() {
        return Err("destination_path must be absolute".into());
    }
    let destination = realpath(Path::new(destination))?;
    let rename = item
        .get("destination_item")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty());
    if let Some(rename) = rename {
        relative(rename, "destination_item")?;
    }
    let requested_name = rename.unwrap_or(source);
    // str's basename behavior retains '.' so it cannot collapse to the directory.
    let name = requested_name.rsplit('/').next().unwrap_or("");
    if name.is_empty() {
        return Err("Destination item name is required".into());
    }
    if name == "." {
        return Err("Destination item name may not be \".\"".into());
    }
    let target = destination.join(name);
    Ok((source_path, destination, target))
}
fn items(request: &Dictionary) -> Result<&[Value], String> {
    request
        .get("items_to_copy")
        .ok_or("No items_to_copy in request")?
        .as_array()
        .map(Vec::as_slice)
        .ok_or("items_to_copy must be an array".into())
}
fn verify(request: &Dictionary) -> Result<PathBuf, String> {
    if !request.contains_key("mount_point") {
        return Err("No mount_point in request".into());
    }
    let items = items(request)?;
    let mount = verify_mount(request, &realpath(Path::new("/private/tmp"))?, mountpoint)?;
    for item in items {
        let item = item
            .as_dictionary()
            .ok_or("items_to_copy item must be a dictionary")?;
        for key in ["source_item", "destination_path"] {
            if !item.contains_key(key) {
                return Err(format!("Missing {key} in items_to_copy item"));
            }
        }
        paths(item, &mount)?;
        verify_mode(mode(item)?)?;
    }
    Ok(mount)
}
fn make_destination(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.exists() {
        missing.push(parent.to_owned());
        parent = parent
            .parent()
            .ok_or("Destination has no existing parent")?;
    }
    let metadata = fs::metadata(parent).map_err(|e| e.to_string())?;
    // Python's makedirs applies the supplied mode only to the leaf; all
    // directories still respect the process umask. A later chmod would undo it.
    let create = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::DirBuilder::new()
            .mode(metadata.mode() & 0o7777)
            .create(path)
    };
    create().map_err(|_| {
        format!(
            "There was an IO error in creating the path {}!",
            path.display()
        )
    })?;
    for path in missing.into_iter().rev() {
        lchown(&path, metadata.uid(), metadata.gid())?;
    }
    Ok(())
}
fn copy_item(source: &Path, target: &Path) -> Result<(), String> {
    // Ditto preserves macOS file metadata and internal symlinks. Handle a source
    // symlink directly so a top-level link is never dereferenced.
    if fs::symlink_metadata(source)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        std::os::unix::fs::symlink(fs::read_link(source).map_err(|e| e.to_string())?, target)
            .map_err(|e| e.to_string())?;
        #[cfg(target_os = "macos")]
        {
            let source = crate::common::cpath(source)?;
            let target = crate::common::cpath(target)?;
            let flags = libc::COPYFILE_STAT | libc::COPYFILE_XATTR | libc::COPYFILE_NOFOLLOW;
            if unsafe {
                libc::copyfile(
                    source.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null_mut(),
                    flags,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
    } else {
        command(Command::new("/usr/bin/ditto").arg(source).arg(target))?;
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn remove_quarantine(path: &Path) -> Result<(), String> {
    let path = crate::common::cpath(path)?;
    let key = c"com.apple.quarantine";
    if unsafe { libc::removexattr(path.as_ptr(), key.as_ptr(), 0) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOATTR) {
            return Err(format!("Error removing xattr: {error}"));
        }
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn remove_quarantine(_path: &Path) -> Result<(), String> {
    Err("Quarantine metadata requires macOS".into())
}
pub fn copy(request: &Dictionary, output: &mut impl Write) -> Result<(), String> {
    let mount = verify(request)?;
    for item in items(request)? {
        let item = item.as_dictionary().unwrap();
        let (source, destination, target) = paths(item, &mount)?;
        verify_mode(mode(item)?)?;
        make_destination(&destination)?;
        if fs::symlink_metadata(&target).is_ok() {
            remove(&target)
                .map_err(|e| format!("Error removing existing {}: {e}", target.display()))?;
        }
        writeln!(
            output,
            "STATUS:Copying {} to {}",
            string(item, "source_item")?,
            target.display()
        )
        .map_err(|e| e.to_string())?;
        copy_item(&source, &target).map_err(|e| {
            format!(
                "Error copying {} to {}: {e}",
                source.display(),
                target.display()
            )
        })?;
        let root = Value::String("root".into());
        let admin = Value::String("admin".into());
        let uid = numeric_id(item.get("user").unwrap_or(&root), false, true)?;
        let gid = numeric_id(item.get("group").unwrap_or(&admin), true, true)?;
        walk(&target, &mut |path| lchown(path, uid, gid)).map_err(|e| {
            format!(
                "Error setting owner and group for {}: {e}",
                target.display()
            )
        })?;
        command(
            Command::new("/bin/chmod")
                .args(["-R", mode(item)?])
                .arg(&target),
        )
        .map_err(|_| format!("Error setting mode for {}", target.display()))?;
        remove_quarantine(&target)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn new_destinations_respect_umask_and_intermediate_directory_defaults() {
        const CHILD: &str = "AUTOPKG_HELPER_UMASK_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "copier::tests::new_destinations_respect_umask_and_intermediate_directory_defaults",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "isolated umask test failed:\n{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }

        // This branch runs alone in a subprocess: umask is process-global.
        for (parent_mode, mask, intermediate_mode, leaf_mode) in
            [(0o777, 0o022, 0o755, 0o755), (0o770, 0o002, 0o775, 0o770)]
        {
            let temp = tempfile::tempdir().unwrap();
            let parent = temp.path().join("parent");
            fs::create_dir(&parent).unwrap();
            fs::set_permissions(&parent, fs::Permissions::from_mode(parent_mode)).unwrap();
            let owner = fs::metadata(&parent).unwrap();
            unsafe { libc::umask(mask) };
            let intermediate = parent.join("intermediate");
            let leaf = intermediate.join("destination");
            make_destination(&leaf).unwrap();
            for (path, expected_mode) in [(&intermediate, intermediate_mode), (&leaf, leaf_mode)] {
                let metadata = fs::metadata(path).unwrap();
                assert_eq!(metadata.mode() & 0o7777, expected_mode);
                assert_eq!(metadata.uid(), owner.uid());
                assert_eq!(metadata.gid(), owner.gid());
            }
        }
    }
    #[test]
    fn rejects_path_escapes_and_dangerous_modes() {
        for path in ["../App", "a/../App", "a\\..\\App"] {
            assert!(relative(path, "source_item").is_err());
        }
        for mode in ["4755", "2755", "u+s", "g=rwxs", "a+S"] {
            assert!(verify_mode(mode).is_err());
        }
        for mode in ["0755", "o-w", "u-s", "g=rw"] {
            verify_mode(mode).unwrap();
        }
    }
    #[test]
    fn confines_source_and_protects_dot_destination() {
        let temp = tempfile::tempdir().unwrap();
        let mount = temp.path().join("mount");
        fs::create_dir(&mount).unwrap();
        let mount = mount.canonicalize().unwrap();
        fs::write(mount.join("App"), "x").unwrap();
        let mut item = Dictionary::from_iter([
            ("source_item", Value::String("App".into())),
            (
                "destination_path",
                temp.path()
                    .join("destination")
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            ),
            ("destination_item", ".".into()),
        ]);
        assert!(paths(&item, &mount).unwrap_err().contains("may not be"));
        item.remove("destination_item");
        assert!(paths(&item, &mount).is_ok());
        fs::write(temp.path().join("outside"), "x").unwrap();
        std::os::unix::fs::symlink(temp.path().join("outside"), mount.join("escape")).unwrap();
        item.insert("source_item".into(), "escape".into());
        assert!(paths(&item, &mount).unwrap_err().contains("outside"));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn copies_real_metadata_and_preserves_links_without_privilege() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "contents").unwrap();
        fs::set_permissions(source.join("file"), fs::Permissions::from_mode(0o640)).unwrap();
        std::os::unix::fs::symlink("file", source.join("link")).unwrap();
        copy_item(&source, &target).unwrap();
        assert_eq!(fs::read(target.join("file")).unwrap(), b"contents");
        assert_eq!(
            fs::metadata(target.join("file")).unwrap().mode() & 0o777,
            0o640
        );
        assert_eq!(
            fs::read_link(target.join("link")).unwrap(),
            PathBuf::from("file")
        );
        let link = temp.path().join("link-copy");
        copy_item(&source.join("link"), &link).unwrap();
        assert_eq!(fs::read_link(link).unwrap(), PathBuf::from("file"));
    }
    #[test]
    fn mount_requires_kernel_mount_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let mount = temp.path().join("mount");
        fs::create_dir(&mount).unwrap();
        let resolved = mount.canonicalize().unwrap();
        let request = Dictionary::from_iter([(
            "mount_point",
            Value::String(resolved.to_string_lossy().into_owned()),
        )]);
        assert!(verify_mount(&request, temp.path(), |_| false).is_err());
        assert_eq!(
            verify_mount(&request, &temp.path().canonicalize().unwrap(), |_| true).unwrap(),
            resolved
        );
    }
}
