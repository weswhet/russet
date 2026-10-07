//! Metadata-only staging support. No installer executable is invoked.
//! Matches pinned Munki shared/osinstaller/osinstallerinfo.swift.
use plist::{Dictionary, Value};
use std::path::Path;

fn app_info(app: &Path) -> Result<(String, Vec<String>), String> {
    let legacy = app.join("Contents/SharedSupport/InstallInfo.plist");
    if legacy.is_file() {
        let plist = Value::from_file(legacy).map_err(|e| e.to_string())?;
        let version = plist
            .as_dictionary()
            .and_then(|d| d.get("System Image Info"))
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("version"))
            .and_then(Value::as_string)
            .ok_or("Could not get version from InstallInfo.plist")?;
        return Ok((version.into(), Vec::new()));
    }
    let image = app.join("Contents/SharedSupport/SharedSupport.dmg");
    if image.is_file() {
        let mut mount = crate::mount::Mount::new(&image.to_string_lossy())?;
        let path = mount.resolve(
            "com_apple_MobileAsset_MacSoftwareUpdate/com_apple_MobileAsset_MacSoftwareUpdate.xml",
        )?;
        let plist = Value::from_file(path).map_err(|e| e.to_string())?;
        let assets = plist
            .as_dictionary()
            .and_then(|d| d.get("Assets"))
            .and_then(Value::as_array)
            .ok_or("Could not parse OS installer assets")?;
        let version = assets
            .first()
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("OSVersion"))
            .and_then(Value::as_string)
            .ok_or("Could not parse OS installer version")?
            .to_owned();
        let models = assets
            .iter()
            .filter_map(Value::as_dictionary)
            .filter_map(|d| d.get("SupportedDeviceModels").and_then(Value::as_array))
            .flatten()
            .filter_map(Value::as_string)
            .map(str::to_owned)
            .collect();
        mount.detach()?;
        return Ok((version, models));
    }
    Err(format!(
        "Could not parse OS installer info from {}",
        app.display()
    ))
}
fn size(path: &Path) -> Result<u64, String> {
    let mut total = 0;
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            total += size(&entry.path())?;
        } else if kind.is_file() {
            total += entry.metadata().map_err(|e| e.to_string())?.len();
        }
    }
    Ok(total)
}
pub(crate) fn stage_metadata(app: &Path) -> Result<Dictionary, String> {
    let (version, models) = app_info(app)?;
    let staged = app.file_stem().unwrap_or_default().to_string_lossy();
    let macos = staged.replace("Install ", "");
    let installed_staged = if version.starts_with("11.") {
        37_224_448u64
    } else {
        27_262_976u64
    };
    let mut info = Dictionary::from_iter([
        (
            "description",
            Value::String(format!("Downloads {macos} installer")),
        ),
        (
            "description_staged",
            Value::String(format!("Installs {macos}, version {version}")),
        ),
        ("display_name", Value::String(format!("{macos} Installer"))),
        ("display_name_staged", Value::String(staged.into_owned())),
        ("installed_size", Value::Integer((size(app)? / 1024).into())),
        (
            "installed_size_staged",
            Value::Integer(installed_staged.into()),
        ),
        ("installer_type", "stage_os_installer".into()),
        ("minimum_munki_version", "6.0.0".into()),
        ("minimum_os_version", "10.9".into()),
        (
            "name",
            Value::String(
                app.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .replace(' ', "_"),
            ),
        ),
        ("uninstallable", true.into()),
        ("version", Value::String(version)),
    ]);
    if !models.is_empty() {
        let boards: Vec<_> = models
            .iter()
            .filter(|s| s.starts_with("Mac-"))
            .cloned()
            .collect();
        let devices: Vec<_> = models
            .iter()
            .filter(|s| !s.starts_with("Mac-"))
            .cloned()
            .collect();
        let mut predicates = Vec::new();
        if !boards.is_empty() {
            predicates.push(format!("board_id IN {{{}}}", boards.join(", ")));
        }
        if !devices.is_empty() {
            predicates.push(format!("device_id IN {{{}}}", devices.join(", ")));
        }
        info.insert(
            "installable_condition_disabled".into(),
            Value::String(predicates.join(" OR ")),
        );
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_installer_metadata_never_executes_startosinstall() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Install macOS Fixture.app");
        std::fs::create_dir_all(app.join("Contents/SharedSupport")).unwrap();
        Value::Dictionary(Dictionary::from_iter([(
            "System Image Info",
            Value::Dictionary(Dictionary::from_iter([(
                "version",
                Value::String("12.6".into()),
            )])),
        )]))
        .to_file_xml(app.join("Contents/SharedSupport/InstallInfo.plist"))
        .unwrap();
        let info = stage_metadata(&app).unwrap();
        assert_eq!(info["name"].as_string(), Some("Install_macOS_Fixture"));
        assert_eq!(info["minimum_os_version"].as_string(), Some("10.9"));
        assert_eq!(
            info["installed_size_staged"].as_unsigned_integer(),
            Some(27_262_976)
        );
    }
}
