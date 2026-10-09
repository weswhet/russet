//! `PkgRootCreator`: create a package root and its folders with the given
//! modes.
use super::Output;
use crate::{io, mode, remove, string, visible_path, Result};
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let root = PathBuf::from(string(env, "pkgroot")?);
    if fs::symlink_metadata(&root).is_ok() {
        remove(&root)?;
    }
    io(fs::create_dir_all(&root))?;
    output(env, 1, format!("Created {}", root.display()));
    let root = io(root.canonicalize())?;
    let dirs = env
        .get("pkgdirs")
        .and_then(Value::as_dictionary)
        .ok_or("pkgdirs must be a dictionary")?;
    let mut sorted: Vec<_> = dirs.iter().collect();
    sorted.sort_by_key(|(k, _)| *k);
    for (dir, permissions) in sorted {
        output(env, 2, format!("Creating {dir}"));
        let mut relative = PathBuf::new();
        for part in Path::new(dir).components() {
            match part {
                Component::Normal(p) => relative.push(p),
                Component::CurDir => (),
                Component::ParentDir => {
                    if !relative.pop() {
                        return Err(format!("{dir} is outside pkgroot"));
                    }
                }
                _ => return Err(format!("{dir} in pkgroot is absolute.")),
            }
        }
        if relative.as_os_str().is_empty() {
            return Err(format!("{dir} is outside pkgroot"));
        }
        let path = root.join(relative);
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        io(fs::create_dir_all(&path))?;
        mode(
            &path,
            permissions
                .as_string()
                .ok_or("pkgdirs modes must be strings")?,
        )?;
        output(env, 1, format!("Created {}", visible_path(&path)));
    }
    Ok(())
}
