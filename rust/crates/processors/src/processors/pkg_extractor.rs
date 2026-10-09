//! `PkgExtractor`: extract a bundle package's Archive.pax.gz into a folder,
//! at its default install location.
use crate::package::{native, run};
use crate::{io, read_dict, string, Result};
use autopkg_platform::backend::{select, Backend, Tool};
use plist::Dictionary;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn execute(env: &Dictionary) -> Result<()> {
    let backend = select(Tool::Ditto);
    if backend == Backend::Unsupported {
        return Err(
            "Bundle package extraction is only supported on macOS and Linux; ditto is unavailable"
                .into(),
        );
    }
    let package = Path::new(string(env, "pkg_path")?);
    let info = package.join("Contents/Info.plist");
    let archive = package.join("Contents/Archive.pax.gz");
    if !info.exists() {
        return Err("Info.plist not found in pkg".into());
    }
    if !archive.exists() {
        return Err("Archive.pax.gz not found in pkg".into());
    }
    let info = read_dict(&info)?;
    let location = info
        .get("IFPkgFlagDefaultLocation")
        .map(|v| {
            v.as_string()
                .ok_or("IFPkgFlagDefaultLocation must be a string")
        })
        .transpose()?
        .unwrap_or("/");
    let mut relative = PathBuf::new();
    for part in Path::new(location.trim_start_matches('/')).components() {
        match part {
            std::path::Component::Normal(p) => relative.push(p),
            std::path::Component::CurDir => (),
            std::path::Component::ParentDir => {
                if !relative.pop() {
                    return Err(format!(
                        "IFPkgFlagDefaultLocation {location:?} resolves outside extract_root"
                    ));
                }
            }
            _ => return Err("IFPkgFlagDefaultLocation resolves outside extract_root".into()),
        }
    }
    let root = Path::new(string(env, "extract_root")?);
    let destination = root.join(relative);
    // Validate existing symlink ancestors before replacing the root.
    if root.exists() {
        let realroot = io(root.canonicalize())?;
        for ancestor in destination.ancestors() {
            if ancestor.exists() {
                if !io(ancestor.canonicalize())?.starts_with(&realroot) {
                    return Err("IFPkgFlagDefaultLocation resolves outside extract_root".into());
                }
                break;
            }
        }
        io(fs::remove_dir_all(root))?;
    }
    io(fs::create_dir_all(&destination))?;
    crate::mode(&destination, "755")?;
    let archive = archive.to_str().ok_or("Invalid package path")?;
    let destination = destination.to_str().ok_or("Invalid extraction path")?;
    if backend == Backend::Native {
        return native::extract_payload(archive, destination);
    }
    run("/usr/bin/ditto", &["-x", "-z", archive, destination])
}
