//! `Installer`: install a package through the `russet-installd` helper on
//! macOS, unless the recipe produced no new package or download.
//!
//! Inputs and outputs: run `russet processor-info Installer`, or see
//! `Installer` in `compatibility/reference.json`.
use crate::clients::{install_summary, mac};
use crate::{matches, string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::Dictionary;

pub(crate) fn skip(env: &mut Dictionary) -> bool {
    env.remove("installer_summary_result");
    let should_skip = if env.contains_key("new_package_request") {
        !truth(env.get("new_package_request"))
    } else {
        env.contains_key("download_changed") && !truth(env.get("download_changed"))
    };
    if should_skip {
        output(
            1,
            if env.contains_key("new_package_request") {
                "Skipping installation: no new package."
            } else {
                "Skipping installation: no new download."
            },
        );
        env.insert("install_result".into(), "SKIPPED".into());
    }
    should_skip
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    mac()?;
    if skip(env) {
        return Ok(());
    }
    let matches = matches(string(env, "pkg_path")?)?;
    crate::package::log_glob("pkg_path", string(env, "pkg_path")?, &matches);
    let package = matches
        .first()
        .ok_or("Error processing pkg_path with glob")?;
    let mut request = Dictionary::new();
    request.insert(
        "package".into(),
        package.to_string_lossy().into_owned().into(),
    );
    request.insert(
        "recipe_cache_dir".into(),
        string(env, "RECIPE_CACHE_DIR")?.into(),
    );
    let result =
        autopkg_helpers::installation_request(&request).unwrap_or_else(|e| format!("ERROR: {e}"));
    output(1, format!("Result: {result}"));
    env.insert("install_result".into(), result.clone().into());
    if result == "DONE" {
        install_summary(
            env,
            "installer_summary_result",
            "The following pkgs were successfully installed:",
            "pkg_path",
            package.to_string_lossy().into_owned().into(),
        );
    }
    Ok(())
}
