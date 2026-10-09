//! `FlatPkgUnpacker`: expand a flat package into a folder, natively or with
//! `pkgutil --expand`, or extract everything but its payload with `xar`.
//!
//! Inputs and outputs: run `russet processor-info FlatPkgUnpacker`, or see
//! `FlatPkgUnpacker` in `compatibility/reference.json`.
use crate::package::{native, run, unsupported};
use crate::{io, portable_path, remove, string, truth, Result};
use autopkg_platform::backend::{select, Backend, Tool};
use autopkg_platform::processor_output as output;
use plist::Dictionary;
use std::{fs, path::Path};

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let source = string(env, "flat_pkg_path")?;
    portable_path(source)?;
    let destination = string(env, "destination_path")?;
    let path = Path::new(destination);
    if !path.exists() {
        io(fs::create_dir_all(path))?;
    } else if truth(env.get("purge_destination")) {
        for entry in io(fs::read_dir(path))? {
            remove(&io(entry)?.path())?;
        }
    }
    if truth(env.get("skip_payload")) {
        match select(Tool::Xar) {
            Backend::Apple => run(
                "/usr/bin/xar",
                &[
                    "-x",
                    "-C",
                    destination,
                    "-f",
                    source,
                    "--exclude",
                    "Payload",
                ],
            ),
            Backend::Native => native::extract_without_payload(source, destination),
            Backend::Unsupported => Err(unsupported("FlatPkgUnpacker")),
        }
    } else {
        if path.exists() {
            io(fs::remove_dir_all(path))?;
        }
        match select(Tool::Pkgutil) {
            Backend::Apple => run("/usr/sbin/pkgutil", &["--expand", source, destination]),
            Backend::Native => native::expand(source, destination),
            Backend::Unsupported => Err(unsupported("FlatPkgUnpacker")),
        }
    }?;
    output(1, format!("Unpacked {source} to {destination}"));
    Ok(())
}
