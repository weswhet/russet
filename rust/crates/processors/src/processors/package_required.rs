//! `PackageRequired`: fail unless the run was given an existing package or
//! disk image with `-p`.
use crate::{string, Result};
use plist::Dictionary;
use std::path::Path;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let pkg = string(env, "PKG").map_err(|_| "This recipe requires a package or disk image to be pre-downloaded and supplied to autopkg (\"-p\" command-line switch). This is likely due to required login credentials to download the software.".to_string())?;
    if pkg.is_empty() || !Path::new(pkg).exists() {
        return Err(format!(
            "Path to package or disk image does not exist: {pkg}"
        ));
    }
    Ok(())
}
