//! `FlatPkgPacker`: flatten an expanded package folder into a flat package,
//! natively or with `pkgutil --flatten`.
//!
//! Inputs and outputs: run `russet processor-info FlatPkgPacker`, or see
//! `FlatPkgPacker` in `compatibility/reference.json`.
use crate::package::{native, run, unsupported};
use crate::{string, Result};
use autopkg_platform::backend::{select, Backend, Tool};
use autopkg_platform::processor_output as output;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let (source, destination) = (
        string(env, "source_flatpkg_dir")?,
        string(env, "destination_pkg")?,
    );
    match select(Tool::Pkgutil) {
        Backend::Apple => run("/usr/sbin/pkgutil", &["--flatten", source, destination])?,
        Backend::Native => native::flatten(source, destination)?,
        Backend::Unsupported => return Err(unsupported("FlatPkgPacker")),
    }
    output(
        1,
        format!(
            "Flattened {} to {}",
            string(env, "source_flatpkg_dir")?,
            string(env, "destination_pkg")?
        ),
    );
    Ok(())
}
