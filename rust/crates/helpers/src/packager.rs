use crate::common::{command, lchmod, lchown, numeric_id, octal, remove, string, walk};
use plist::{Dictionary, Value};
use regex::Regex;
use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

fn directory_owned(path: &Path, uid: u32) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("Can't stat {}: {e}", path.display()))?;
    if metadata.uid() != uid {
        return Err(format!("{} isn't owned by {uid}", path.display()));
    }
    if metadata.file_type().is_symlink() {
        return Err(format!("{} is a soft link", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    Ok(())
}
fn ownerships_enabled(path: &Path) -> bool {
    let Ok(output) = Command::new("/usr/sbin/diskutil")
        .args(["list", "-plist"])
        .output()
    else {
        return true;
    };
    let Ok(data) = Value::from_reader(std::io::Cursor::new(output.stdout)) else {
        return true;
    };
    let Some(disks) = data
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(Value::as_array)
    else {
        return true;
    };
    let mut mounts = Vec::new();
    for disk in disks {
        let Some(disk) = disk.as_dictionary() else {
            continue;
        };
        if let Some(mount) = disk.get("MountPoint").and_then(Value::as_string) {
            mounts.push(mount);
        }
        if let Some(partitions) = disk.get("Partitions").and_then(Value::as_array) {
            for partition in partitions {
                if let Some(mount) = partition
                    .as_dictionary()
                    .and_then(|d| d.get("MountPoint"))
                    .and_then(Value::as_string)
                {
                    mounts.push(mount);
                }
            }
        }
    }
    mounts.sort_by_key(|mount| *mount == "/");
    let Ok(real) = path.canonicalize() else {
        return true;
    };
    let Some(mount) = mounts.into_iter().find(|m| real.starts_with(m)) else {
        return true;
    };
    let Ok(output) = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist", mount])
        .output()
    else {
        return true;
    };
    Value::from_reader(std::io::Cursor::new(output.stdout))
        .ok()
        .and_then(|v| {
            v.as_dictionary()?
                .get("GlobalPermissionsEnabled")?
                .as_boolean()
        })
        .unwrap_or(true)
}
fn verify_basic(request: &Dictionary, uid: u32) -> Result<(), String> {
    directory_owned(Path::new(string(request, "pkgroot")?), uid)?;
    directory_owned(Path::new(string(request, "pkgdir")?), uid)?;
    let name = string(request, "pkgname")?;
    if name.chars().count() > 80 {
        return Err("Package name too long".into());
    }
    if !Regex::new(r"(?i)^[a-z0-9][a-z0-9 ._\-]*$")
        .unwrap()
        .is_match(name)
    {
        return Err("Invalid package name".into());
    }
    if name.to_lowercase().ends_with(".pkg") {
        return Err("Package name mustn't include '.pkg'".into());
    }
    let id = string(request, "id")?;
    if id.chars().count() > 80 {
        return Err("Package id too long".into());
    }
    let re = Regex::new(r"(?i)^[a-z0-9]([a-z0-9 \-]*[a-z0-9])?$").unwrap();
    if id.split('.').count() < 2 || !id.split('.').all(|part| re.is_match(part)) {
        return Err("Invalid package id".into());
    }
    let version = string(request, "version")?;
    if version.chars().count() > 40 {
        return Err("Version too long".into());
    }
    let re = Regex::new(r"(?i)^[a-z0-9_ ]*[0-9][a-z0-9_ -]*$").unwrap();
    for part in version.split('.') {
        if !re.is_match(part) {
            return Err(format!("Invalid version component \"{part}\""));
        }
    }
    let info = string(request, "infofile")?;
    if !info.is_empty() {
        fs::File::open(info).map_err(|e| format!("Can't open infofile: {e}"))?;
    }
    let scripts = string(request, "scripts")?;
    if !scripts.is_empty() {
        if !Path::new(scripts).is_dir() {
            return Err(format!("Can't find scripts directory: {scripts}"));
        }
        for name in ["preinstall", "postinstall"] {
            let path = Path::new(scripts).join(name);
            if path.exists() && fs::metadata(path).map_err(|e| e.to_string())?.mode() & 0o111 == 0 {
                return Err(format!(
                    "{name} script found in {scripts} but it is not executable!"
                ));
            }
        }
    }
    Ok(())
}
fn chown_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("Empty chown path".into());
    }
    let mut result = root.to_owned();
    for part in path.split('/') {
        if part == "." || part == ".." {
            return Err(". and .. is not allowed in chown path".into());
        }
        if part.is_empty() {
            continue;
        }
        result.push(part);
        let relative = result.strip_prefix(root).unwrap().display();
        let metadata = fs::symlink_metadata(&result)
            .map_err(|_| format!("chown path {relative} does not exist"))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("chown path {relative} is a soft link"));
        }
    }
    Ok(result)
}
fn apply_chown(request: &Dictionary, root: &Path) -> Result<(), String> {
    let entries = request
        .get("chown")
        .and_then(Value::as_array)
        .ok_or("chown must be an array")?;
    for entry in entries {
        let entry = entry
            .as_dictionary()
            .ok_or("chown entry isn't dictionary")?;
        let path = chown_path(root, string(entry, "path")?)?;
        let uid = numeric_id(
            entry.get("user").ok_or("chown entry is missing user")?,
            false,
            false,
        )
        .map_err(|e| e.replace("Unknown user", "Unknown chown user"))?;
        let gid = numeric_id(
            entry.get("group").ok_or("chown entry is missing group")?,
            true,
            false,
        )
        .map_err(|e| e.replace("Unknown group", "Unknown chown group"))?;
        let mode = entry
            .get("mode")
            .map(|v| {
                v.as_string()
                    .ok_or("chown mode must be a string")
                    .and_then(|s| octal(s).map_err(|_| "Invalid chown mode"))
            })
            .transpose()?;
        if path.is_file() {
            lchown(&path, uid, gid)?;
            if let Some(mode) = mode {
                lchmod(&path, mode)?;
            }
        } else {
            // Reference applies ownership to the root directory, but mode only
            // to descendants (each directory entry can be visited twice).
            walk(&path, &mut |child| {
                lchown(child, uid, gid)?;
                if child != path {
                    if let Some(mode) = mode {
                        lchmod(child, mode)?;
                    }
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}
fn component_plist(root: &Path, path: &Path) -> Result<(), String> {
    command(
        Command::new("/usr/bin/pkgbuild")
            .args(["--analyze", "--root"])
            .arg(root)
            .arg(path),
    )?;
    let mut data =
        Value::from_file(path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    for item in data
        .as_array_mut()
        .ok_or("Component plist must be an array")?
    {
        let item = item
            .as_dictionary_mut()
            .ok_or("Component entry must be a dictionary")?;
        if item.get("BundleIsRelocatable").and_then(Value::as_boolean) == Some(true) {
            item.insert("BundleIsRelocatable".into(), false.into());
        }
    }
    data.to_file_xml(path)
        .map_err(|e| format!("Couldn't write {}: {e}", path.display()))
}
fn build_command(
    request: &Dictionary,
    root: &Path,
    component: &Path,
    output: &Path,
) -> Result<Command, String> {
    let mut command = Command::new("/usr/bin/pkgbuild");
    command
        .arg("--root")
        .arg(root)
        .args([
            "--identifier",
            string(request, "id")?,
            "--version",
            string(request, "version")?,
            "--ownership",
            "preserve",
            "--component-plist",
        ])
        .arg(component);
    for (key, flag) in [("infofile", "--info"), ("scripts", "--scripts")] {
        let value = string(request, key)?;
        if !value.is_empty() {
            command.args([flag, value]);
        }
    }
    if let Some(args) = request.get("pkgbuild_args") {
        for arg in args.as_array().ok_or("pkgbuild_args must be an array")? {
            command.arg(
                arg.as_string()
                    .ok_or("pkgbuild_args entry is not a string")?,
            );
        }
    }
    command.arg(output);
    Ok(command)
}
pub fn package(request: &Dictionary, uid: u32, gid: u32) -> Result<PathBuf, String> {
    let root = Path::new(string(request, "pkgroot")?);
    if !ownerships_enabled(root) {
        return Err(format!("'Ignore ownerships' is set on the disk where pkgroot '{}' was set, and packaging cannot continue. Ownerships must be enabled on the volume where a package is to be built.", root.display()));
    }
    verify_basic(request, uid)?;
    let temporary = tempfile::tempdir().map_err(|e| e.to_string())?;
    let copy = temporary
        .path()
        .join(root.file_name().ok_or("Invalid pkgroot name")?);
    fs::create_dir(&copy).map_err(|e| e.to_string())?;
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o1775)).map_err(|e| e.to_string())?;
    lchown(&copy, 0, 80)?;
    command(Command::new("/usr/bin/ditto").arg(root).arg(&copy))?;
    apply_chown(request, &copy)?;
    let component = temporary.path().join("component.plist");
    component_plist(&copy, &component)?;
    if string(request, "pkgtype")? != "flat" {
        return Err(format!(
            "Unsupported pkgtype {}",
            string(request, "pkgtype")?
        ));
    }
    let name = format!("{}.pkg", string(request, "pkgname")?);
    let output = Path::new(string(request, "pkgdir")?).join(&name);
    if output.exists() {
        if fs::symlink_metadata(&output)
            .map_err(|e| e.to_string())?
            .uid()
            != uid
        {
            return Err(format!(
                "Existing pkg {} not owned by {uid}",
                output.display()
            ));
        }
        remove(&output)
            .map_err(|e| format!("Can't remove existing pkg {}: {e}", output.display()))?;
    }
    let mut random = [0u8; 8];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut random))
        .map_err(|e| e.to_string())?;
    let random = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let temporary_output =
        Path::new(string(request, "pkgdir")?).join(format!("autopkgtmp-{random}-{name}"));
    let result = (|| {
        command(&mut build_command(
            request,
            &copy,
            &component,
            &temporary_output,
        )?)?;
        fs::rename(&temporary_output, &output).map_err(|e| e.to_string())?;
        lchown(&output, uid, gid)?;
        Ok(output)
    })();
    if let Err(error) = fs::remove_file(&temporary_output) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!(
                "Can't remove temporary package {}: {error}",
                temporary_output.display()
            );
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(root: &Path) -> Dictionary {
        Dictionary::from_iter([
            ("pkgroot", root.to_string_lossy().into_owned().into()),
            ("pkgdir", root.to_string_lossy().into_owned().into()),
            ("pkgname", "App".into()),
            ("pkgtype", "flat".into()),
            ("id", "org.app".into()),
            ("version", "1.0".into()),
            ("infofile", "".into()),
            ("scripts", "".into()),
            ("chown", Value::Array(vec![])),
        ])
    }
    #[test]
    fn ownership_names_and_symlinks_are_checked_without_privilege() {
        let temp = tempfile::tempdir().unwrap();
        let uid = fs::metadata(temp.path()).unwrap().uid();
        let mut req = request(temp.path());
        verify_basic(&req, uid).unwrap();
        assert!(verify_basic(&req, uid.wrapping_add(1))
            .unwrap_err()
            .contains("isn't owned"));
        req.insert("pkgname".into(), "App.pkg".into());
        assert!(verify_basic(&req, uid)
            .unwrap_err()
            .contains("mustn't include"));
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(temp.path(), &link).unwrap();
        assert!(directory_owned(&link, uid)
            .unwrap_err()
            .contains("soft link"));
    }
    #[test]
    fn chown_paths_cannot_escape_or_follow_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("dir")).unwrap();
        assert_eq!(
            chown_path(temp.path(), "/dir").unwrap(),
            temp.path().join("dir")
        );
        assert!(chown_path(temp.path(), "../outside").is_err());
        assert!(chown_path(temp.path(), "dir/.").is_err());
        std::os::unix::fs::symlink("dir", temp.path().join("link")).unwrap();
        assert!(chown_path(temp.path(), "link")
            .unwrap_err()
            .contains("soft link"));
    }
    #[test]
    fn pkgbuild_arguments_preserve_order_before_output() {
        let mut req = request(Path::new("/tmp/root"));
        req.insert(
            "pkgbuild_args".into(),
            vec![
                Value::String("--min-os-version".into()),
                Value::String("12.0".into()),
            ]
            .into(),
        );
        let cmd = build_command(
            &req,
            Path::new("/tmp/root"),
            Path::new("/tmp/component.plist"),
            Path::new("/tmp/out.pkg"),
        )
        .unwrap();
        let args: Vec<_> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();
        assert_eq!(
            &args[args.len() - 3..],
            &["--min-os-version", "12.0", "/tmp/out.pkg"]
        );
    }
}
