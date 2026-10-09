//! `SassafrasK2ClientCustomizer`: customize a Sassafras K2 client package.
//! A native port of the autopkg/recipes processor (Apache-2.0).
use crate::community_legacy::Result;
use plist::Dictionary;
use std::{fs, path::Path, process::Command};

pub(crate) fn execute(env: &Dictionary) -> Result<()> {
    let script = crate::string(env, "k2clientconfig_path")?;
    let pkg = crate::string(env, "base_pkg_path")?;
    if !Path::new(script).exists() {
        return Err(format!("No file exists at k2clientconfig_path: {script}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(script)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            fs::set_permissions(script, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }
    if !Path::new(pkg).exists() {
        return Err(format!("No K2Client pkg exists at base_pkg_path: {pkg}"));
    }
    let result = Command::new(script)
        .args(crate::string(env, "k2clientconfig_options")?.split_whitespace())
        .arg(pkg)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.stderr.is_empty() {
        return Err(format!(
            "k2clientconfig returned errors:\n{}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}
