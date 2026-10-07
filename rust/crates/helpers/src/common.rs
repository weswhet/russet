use plist::{Dictionary, Value};
use std::{
    ffi::CString,
    fs, io,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    process::Command,
};

pub fn string<'a>(request: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    request
        .get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("No {key} in request"))
}
pub fn realpath(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    if let Ok(path) = absolute.canonicalize() {
        return Ok(path);
    }
    let mut parent = absolute.as_path();
    let mut suffix = Vec::new();
    while !parent.exists() {
        if let Some(name) = parent.file_name() {
            suffix.push(name.to_owned());
        }
        parent = parent
            .parent()
            .ok_or_else(|| format!("Cannot resolve {}", path.display()))?;
    }
    let mut resolved = parent.canonicalize().map_err(|e| e.to_string())?;
    for name in suffix.into_iter().rev() {
        resolved.push(name);
    }
    let mut clean = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::ParentDir => {
                clean.pop();
            }
            Component::CurDir => {}
            other => clean.push(other.as_os_str()),
        }
    }
    Ok(clean)
}
pub fn mountpoint(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if metadata.file_type().is_symlink() {
        return false;
    }
    let parent = path.parent().unwrap_or(path);
    let Ok(parent_metadata) = fs::metadata(parent) else {
        return false;
    };
    metadata.dev() != parent_metadata.dev() || metadata.ino() == parent_metadata.ino()
}
pub fn cpath(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| "Path contains a NUL byte".into())
}
pub fn lchown(path: &Path, uid: u32, gid: u32) -> Result<(), String> {
    let path = cpath(path)?;
    if unsafe { libc::lchown(path.as_ptr(), uid, gid) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().to_string())
    }
}
pub fn lchmod(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let path = cpath(path)?;
        if unsafe { libc::lchmod(path.as_ptr(), mode as libc::mode_t) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error().to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Symlink chmod requires macOS".into());
        }
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())
    }
}
pub fn remove(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| e.to_string())
}
pub fn numeric_id(value: &Value, group: bool, accept_numeric_string: bool) -> Result<u32, String> {
    let kind = if group { "group" } else { "user" };
    match value {
        Value::Integer(number) => number
            .as_unsigned()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| format!("Invalid {kind} id {number}")),
        Value::Boolean(value) => Ok(u32::from(*value)),
        Value::String(name) => {
            if accept_numeric_string && !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
            {
                return name
                    .parse()
                    .map_err(|_| format!("Invalid {kind} id {name}"));
            }
            let name_c =
                CString::new(name.as_str()).map_err(|_| format!("Unknown {kind} {name}"))?;
            // Workers are sequential; libc's account lookup storage is read immediately.
            unsafe {
                if group {
                    let result = libc::getgrnam(name_c.as_ptr());
                    if result.is_null() {
                        Err(format!("Unknown {kind} {name}"))
                    } else {
                        Ok((*result).gr_gid)
                    }
                } else {
                    let result = libc::getpwnam(name_c.as_ptr());
                    if result.is_null() {
                        Err(format!("Unknown {kind} {name}"))
                    } else {
                        Ok((*result).pw_uid)
                    }
                }
            }
        }
        _ => Err(format!("Invalid {kind} id")),
    }
}
pub fn octal(mode: &str) -> Result<u32, String> {
    u32::from_str_radix(mode, 8).map_err(|_| format!("Invalid octal mode {mode}"))
}
pub fn command(command: &mut Command) -> Result<std::process::Output, String> {
    let name = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .map_err(|e| format!("{name} execution failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{name} failed with exit code {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    Ok(output)
}
pub fn walk(
    path: &Path,
    action: &mut impl FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    action(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        for item in fs::read_dir(path).map_err(|e| e.to_string())? {
            walk(&item.map_err(|e| e.to_string())?.path(), action)?;
        }
    }
    Ok(())
}
