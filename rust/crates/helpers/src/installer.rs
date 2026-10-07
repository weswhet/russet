use crate::common::{mountpoint, realpath, string};
use plist::Dictionary;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn allowed(path: &Path, cache: &Path, temporary: &Path, mounted: impl Fn(&Path) -> bool) -> bool {
    if path.starts_with(cache) {
        return true;
    }
    if !path.starts_with(temporary) {
        return false;
    }
    let mut ancestor = if path.is_dir() {
        Some(path)
    } else {
        path.parent()
    };
    while let Some(path) = ancestor {
        if !path.starts_with(temporary) {
            break;
        }
        if path != temporary && mounted(path) {
            return true;
        }
        ancestor = path.parent();
    }
    false
}
pub fn verify(request: &Dictionary) -> Result<PathBuf, String> {
    for key in ["package", "recipe_cache_dir"] {
        if !request.contains_key(key) {
            return Err(format!("ERROR:No {key} in request"));
        }
    }
    let package = string(request, "package").map_err(|_| "Package path is required")?;
    let cache =
        string(request, "recipe_cache_dir").map_err(|_| "Recipe cache directory is required")?;
    if package.is_empty() {
        return Err("Package path is required".into());
    }
    if cache.is_empty() {
        return Err("Recipe cache directory is required".into());
    }
    let package_path = realpath(Path::new(package))?;
    let cache = realpath(Path::new(cache))?;
    let temporary = realpath(Path::new("/private/tmp"))?;
    if !allowed(&package_path, &cache, &temporary, mountpoint) {
        return Err(format!(
            "Package path {package} is not in an allowed location"
        ));
    }
    if !package_path.exists() {
        return Err(format!("Package path {package} does not exist"));
    }
    Ok(package_path)
}
pub fn install(request: &Dictionary, output: &mut impl Write) -> Result<(), String> {
    let package = verify(request)?;
    // A pipe merges stdout/stderr in the same order as stderr=STDOUT in Python.
    let (mut read, write) = std::os::unix::net::UnixStream::pair().map_err(|e| e.to_string())?;
    let stdout: std::os::fd::OwnedFd = write.try_clone().map_err(|e| e.to_string())?.into();
    let stderr: std::os::fd::OwnedFd = write.into();
    let mut child = Command::new("/usr/sbin/installer")
        .args(["-verboseR", "-pkg"])
        .arg(package)
        .args(["-target", "/"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| format!("ERROR:{e}\n"))?;
    for line in BufReader::new(&mut read).split(b'\n') {
        let line = line.map_err(|e| e.to_string())?;
        output.write_all(b"STATUS:").map_err(|e| e.to_string())?;
        output.write_all(&line).map_err(|e| e.to_string())?;
        output.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("ERROR:{}\n", status.code().unwrap_or(-1)));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confines_packages_and_requires_actual_temporary_mount() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let tmp = temp.path().join("tmp");
        assert!(allowed(&cache.join("file.pkg"), &cache, &tmp, |_| false));
        assert!(!allowed(
            &temp.path().join("cache-escape/file.pkg"),
            &cache,
            &tmp,
            |_| false
        ));
        assert!(!allowed(&tmp.join("file.pkg"), &cache, &tmp, |_| false));
        assert!(allowed(&tmp.join("mount/file.pkg"), &cache, &tmp, |p| p
            == tmp.join("mount")));
    }
    #[test]
    fn missing_fields_fail_before_starting_installer() {
        assert_eq!(
            verify(&Dictionary::new()).unwrap_err(),
            "ERROR:No package in request"
        );
    }
}
