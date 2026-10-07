//! Package and disk-image queries that call Apple's tools on macOS and
//! Russet's replacements on Linux (or with `RUSSET_NATIVE`).

use crate::metadata::command;
use autopkg_platform::backend::{select, Backend, Tool};
use plist::Value;
use std::path::Path;

const UNSUPPORTED: &str = "Apple package inspection is only supported on macOS and Linux";

fn native<T>(result: std::io::Result<T>) -> Result<T, String> {
    result.map_err(|e| e.to_string())
}

/// Every `PackageInfo` and `Distribution` in a flat package, with its path
/// in the archive, in archive order.
pub(crate) fn metadata_documents(path: &Path) -> Result<Vec<(String, String)>, String> {
    match select(Tool::Xar) {
        Backend::Apple => apple_documents(path),
        #[cfg(unix)]
        Backend::Native => {
            let mut archive = native(russet_xar::Archive::open(path))?;
            let names: Vec<String> = archive
                .entries()
                .iter()
                .filter(|e| e.kind == russet_xar::EntryKind::File)
                .map(|e| e.path.to_string_lossy().into_owned())
                .filter(|p| p.ends_with("PackageInfo") || p.ends_with("Distribution"))
                .collect();
            names
                .into_iter()
                .map(|name| {
                    let bytes = native(archive.read(&name, 16 << 20))?;
                    Ok((name, String::from_utf8(bytes).map_err(|e| e.to_string())?))
                })
                .collect()
        }
        _ => Err(UNSUPPORTED.into()),
    }
}

fn apple_documents(path: &Path) -> Result<Vec<(String, String)>, String> {
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let toc = command("/usr/bin/xar", &["-tf".as_ref(), path.as_os_str()])?;
    let toc = String::from_utf8(toc).map_err(|e| e.to_string())?;
    let mut documents = Vec::new();
    for entry in toc
        .lines()
        .filter(|s| s.ends_with("PackageInfo") || s.ends_with("Distribution"))
    {
        if Path::new(entry).components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            return Err("Package metadata archive path escapes temporary directory".into());
        }
        let output = std::process::Command::new("/usr/bin/xar")
            .arg("-xf")
            .arg(path)
            .arg(entry)
            .current_dir(temp.path())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        let extracted = temp.path().join(entry);
        if !extracted
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(temp.path().canonicalize().map_err(|e| e.to_string())?)
        {
            return Err("Package metadata symlink escapes temporary directory".into());
        }
        documents.push((
            entry.to_owned(),
            std::fs::read_to_string(extracted).map_err(|e| e.to_string())?,
        ));
    }
    Ok(documents)
}

/// The package's restart action, or `None` when it doesn't ask for one.
/// `bundle_flag` is a bundle package's `IFPkgFlagRestartAction`.
pub(crate) fn restart_action(
    path: &Path,
    bundle_flag: Option<&str>,
) -> Result<Option<String>, String> {
    let action = match select(Tool::Installer) {
        Backend::Apple => {
            let output = command(
                "/usr/sbin/installer",
                &[
                    "-query".as_ref(),
                    "RestartAction".as_ref(),
                    "-pkg".as_ref(),
                    path.as_os_str(),
                    "-plist".as_ref(),
                ],
            )?;
            match Value::from_reader(std::io::Cursor::new(output)) {
                Ok(Value::Dictionary(restart)) => restart
                    .get("RestartAction")
                    .and_then(Value::as_string)
                    .map(str::to_owned),
                _ => None,
            }
        }
        #[cfg(unix)]
        Backend::Native => Some(
            match bundle_flag {
                Some(flag) => russet_installer::RestartAction::from_bundle_flag(flag),
                None if path.is_dir() => russet_installer::RestartAction::None,
                None => native(russet_installer::restart_action(path))?,
            }
            .as_str()
            .to_owned(),
        ),
        _ => return Err(UNSUPPORTED.into()),
    };
    Ok(action.filter(|a| a != "None"))
}

/// `installer -showChoiceChangesXML`, filtered to `selected` changes.
pub(crate) fn installer_choices(path: &Path) -> Result<Option<Vec<Value>>, String> {
    match select(Tool::Installer) {
        Backend::Apple => {
            let output = command(
                "/usr/sbin/installer",
                &[
                    "-showChoiceChangesXML".as_ref(),
                    "-pkg".as_ref(),
                    path.as_os_str(),
                ],
            )?;
            Ok(match Value::from_reader(std::io::Cursor::new(output)) {
                Ok(Value::Array(choices)) => Some(
                    choices
                        .into_iter()
                        .filter(|v| {
                            v.as_dictionary()
                                .and_then(|d| d.get("choiceAttribute"))
                                .and_then(Value::as_string)
                                == Some("selected")
                        })
                        .collect(),
                ),
                _ => None,
            })
        }
        #[cfg(unix)]
        Backend::Native => native(russet_installer::choice_changes(path)).map(|_| None),
        _ => Err(UNSUPPORTED.into()),
    }
}

/// The image `Format` that `hdiutil imageinfo` reports, such as `UDZO`.
pub(crate) fn image_format(path: &Path) -> Result<Option<String>, String> {
    match select(Tool::Hdiutil) {
        Backend::Apple => {
            let output = command(
                "/usr/bin/hdiutil",
                &["imageinfo".as_ref(), path.as_os_str(), "-plist".as_ref()],
            )?;
            let image = autopkg_platform::dmg::parse_hdiutil_plist(&output)?;
            Ok(image
                .as_dictionary()
                .and_then(|d| d.get("Format"))
                .and_then(Value::as_string)
                .map(str::to_owned))
        }
        #[cfg(unix)]
        Backend::Native => Ok(Some(native(russet_hdiutil::image_info(path))?.format)),
        _ => Err("Disk image operations are only supported on macOS and Linux".into()),
    }
}
