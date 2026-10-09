//! `InstallFromDMG`: copy items from a disk image into place through the
//! `russet-installd` helper on macOS.
use crate::clients::{install_summary, mac};
use crate::dmg::Mount;
use crate::{string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    mac()?;
    env.remove("install_from_dmg_summary_result");
    if env.contains_key("download_changed") && !truth(env.get("download_changed")) {
        output(1, "Skipping installation: no new download.");
        env.insert("install_result".into(), "SKIPPED".into());
        return Ok(());
    }
    let mut mount = Mount::new(string(env, "dmg_path")?)?;
    let mut request = Dictionary::new();
    request.insert(
        "mount_point".into(),
        mount.path().to_string_lossy().into_owned().into(),
    );
    request.insert(
        "items_to_copy".into(),
        env.get("items_to_copy")
            .ok_or("Missing items_to_copy")?
            .clone(),
    );
    let result =
        autopkg_helpers::installation_request(&request).unwrap_or_else(|e| format!("ERROR: {e}"));
    output(1, format!("Result: {result}"));
    env.insert("install_result".into(), result.clone().into());
    if result == "DONE" {
        install_summary(
            env,
            "install_from_dmg_summary_result",
            "Items from the following disk images were successfully installed:",
            "dmg_path",
            env["dmg_path"].clone(),
        );
    }
    mount.detach()
}
