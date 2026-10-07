//! Cache paths are resolved before creation, including existing symlinks.
use plist::{Dictionary, Value};
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
fn account_home(name: Option<&str>) -> Option<PathBuf> {
    use std::{
        ffi::{CStr, CString},
        os::unix::ffi::OsStrExt,
    };
    let name = name.map(CString::new).transpose().ok()?;
    let mut size = 16384;
    loop {
        let mut buffer = vec![0u8; size];
        let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let code = unsafe {
            match &name {
                Some(name) => libc::getpwnam_r(
                    name.as_ptr(),
                    record.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
                None => libc::getpwuid_r(
                    libc::getuid(),
                    record.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                ),
            }
        };
        if code == libc::ERANGE {
            size = size.checked_mul(2)?;
            if size > 16 * 1024 * 1024 {
                return None;
            }
            continue;
        }
        if code != 0 || result.is_null() {
            return None;
        }
        let record = unsafe { record.assume_init() };
        if record.pw_dir.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes();
        return Some(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)));
    }
}
fn expanded_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if !text.starts_with('~') {
        return path.to_owned();
    }
    let boundary = text
        .find(|c| c == '/' || (cfg!(windows) && c == '\\'))
        .unwrap_or(text.len());
    let username = &text[1..boundary];
    #[cfg(unix)]
    let home = if username.is_empty() {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(|| account_home(None))
    } else {
        account_home(Some(username))
    };
    #[cfg(windows)]
    let home = {
        let current = std::env::var_os("USERPROFILE")
            .or_else(|| {
                std::env::var_os("HOMEPATH").map(|tail| {
                    let mut value = std::env::var_os("HOMEDRIVE").unwrap_or_default();
                    value.push(tail);
                    value
                })
            })
            .map(PathBuf::from);
        current.and_then(|home| {
            if username.is_empty() {
                Some(home)
            } else {
                let current_user = std::env::var_os("USERNAME")?;
                if home.file_name() != Some(current_user.as_os_str()) {
                    return None;
                }
                Some(home.parent()?.join(username))
            }
        })
    };
    #[cfg(not(any(unix, windows)))]
    let home = None::<PathBuf>;
    match home {
        Some(home) => home.join(
            text.get(boundary + 1..)
                .unwrap_or("")
                .trim_start_matches(|c| c == '/' || (cfg!(windows) && c == '\\')),
        ),
        None => path.to_owned(),
    }
}
/// Apply Python's expanduser/abspath cache normalization without filesystem writes.
pub fn normalized(path: &Path) -> Result<PathBuf, String> {
    let expanded = expanded_home(path);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(expanded)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    Ok(result)
}

fn resolve(path: &Path) -> Result<PathBuf, String> {
    let mut resolved = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?
    };
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => {
                resolved.push(name);
                match std::fs::symlink_metadata(&resolved) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        resolved = resolved.canonicalize().map_err(|e| {
                            format!("Could not resolve cache path {}: {e}", resolved.display())
                        })?;
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        return Err(format!(
                            "Could not inspect cache path {}: {e}",
                            resolved.display()
                        ))
                    }
                }
            }
        }
    }
    Ok(resolved)
}

/// Compute a cache path without writing anything. Existing symlinks cannot
/// redirect the recipe outside the supplied root.
pub fn recipe_cache_path(root: &Path, identifier: &str) -> Result<PathBuf, String> {
    if identifier.is_empty() {
        return Err("Recipe identifier cannot be empty".into());
    }
    let root = normalized(root)?;
    let candidate = normalized(&root.join(identifier))?;
    if !resolve(&candidate)?.starts_with(resolve(&root)?) {
        return Err(format!(
            "Recipe identifier {identifier:?} resolves outside CACHE_DIR {}",
            root.display()
        ));
    }
    Ok(candidate)
}

/// Initialize an explicitly configured cache. Engine callers choose the root;
/// the engine never implicitly writes into a user's production cache.
pub fn initialize(env: &mut Dictionary, identifier: &str) -> Result<(), String> {
    let Some(root) = env.get("CACHE_DIR") else {
        return Ok(());
    };
    let root = root.as_string().ok_or("CACHE_DIR must be a path string")?;
    if root.is_empty() {
        return Err("CACHE_DIR cannot be empty".into());
    }
    let root = normalized(Path::new(root))?;
    let path = recipe_cache_path(&root, identifier)?;
    let resolved_before = resolve(&path)?;
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Could not create RECIPE_CACHE_DIR {}: {e}", path.display()))?;
    // Detect a path swapped while creating ordinary cache directories.
    let after = recipe_cache_path(&root, identifier)?;
    if resolve(&after)? != resolved_before {
        return Err("Recipe cache path changed during initialization".into());
    }
    env.insert(
        "CACHE_DIR".into(),
        Value::String(root.to_string_lossy().into_owned()),
    );
    env.insert(
        "RECIPE_CACHE_DIR".into(),
        Value::String(path.to_string_lossy().into_owned()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn named_user_expansion_uses_account_database_without_creating_paths() {
        let user = std::process::Command::new("id")
            .arg("-un")
            .output()
            .unwrap();
        assert!(user.status.success());
        let username = String::from_utf8(user.stdout).unwrap();
        let username = username.trim();
        let home = account_home(Some(username)).unwrap();
        assert_eq!(
            normalized(Path::new(&format!("~{username}/Cache/../Cache"))).unwrap(),
            home.join("Cache")
        );
        assert_eq!(
            normalized(Path::new(&format!("~{username}//Cache"))).unwrap(),
            home.join("Cache")
        );
        let unknown = "~autopkg_no_such_account_a57e9c/Cache";
        assert_eq!(
            normalized(Path::new(unknown)).unwrap(),
            std::env::current_dir().unwrap().join(unknown)
        );
    }
    #[test]
    fn rejects_escape_without_creating_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        assert!(recipe_cache_path(&root, "../escape")
            .unwrap_err()
            .contains("outside CACHE_DIR"));
        assert!(!root.exists());
        let mut env = Dictionary::from_iter([(
            "CACHE_DIR",
            Value::String(root.to_string_lossy().into_owned()),
        )]);
        initialize(&mut env, "org.test").unwrap();
        assert!(root.join("org.test").is_dir());
        assert_eq!(
            env["RECIPE_CACHE_DIR"].as_string(),
            root.join("org.test").to_str()
        );
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_and_allows_internal_links() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        assert!(recipe_cache_path(&root, "escape/child").is_err());
        assert_eq!(
            recipe_cache_path(&root, "alias/child").unwrap(),
            root.join("alias/child")
        );
        assert!(recipe_cache_path(&root, "escape/../escape").is_err());
        assert!(!outside.join("child").exists());
    }
}
