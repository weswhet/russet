//! `AppDmgVersioner`: read an app's bundle identifier and version from the
//! first app in a disk image.
//!
//! Inputs and outputs: run `russet processor-info AppDmgVersioner`, or see
//! `AppDmgVersioner` in `compatibility/reference.json`.
use crate::dmg::Mount;
use crate::{matches, read_dict, string, Result};
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let mut mount = Mount::new(string(env, "dmg_path")?)?;
    let result = (|| {
        let pattern = mount.resolve("*.app")?;
        let paths = matches(&pattern)?;
        let app = paths.first().ok_or("No app found in dmg")?;
        let info = read_dict(&app.join("Contents/Info.plist"))?;
        env.insert(
            "app_name".into(),
            app.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
                .into(),
        );
        let id = info
            .get("CFBundleIdentifier")
            .ok_or("Can't read bundle info: missing CFBundleIdentifier")?;
        env.insert("bundleid".into(), id.clone());
        let version = info
            .get("CFBundleShortVersionString")
            .ok_or("Can't read bundle info: missing CFBundleShortVersionString")?;
        env.insert("version".into(), version.clone());
        autopkg_platform::processor_output(1, format!("BundleID: {}", plist::python_str(id)));
        autopkg_platform::processor_output(1, format!("Version: {}", plist::python_str(version)));
        Ok(())
    })();
    mount.detach().and(result)
}
