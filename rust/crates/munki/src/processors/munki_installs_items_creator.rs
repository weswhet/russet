//! `MunkiInstallsItemsCreator`: generate a pkginfo's installs items, natively,
//! following Munki 7.2.0.5787 createInstallsItem.
//! Reference: <https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/shared/admin/pkginfolib.swift>
//!
//! Inputs and outputs: run `russet processor-info MunkiInstallsItemsCreator`, or see
//! `MunkiInstallsItemsCreator` in `compatibility/reference.json`.
use md5::{Digest, Md5};
use plist::{Dictionary, Value};
use std::{cmp::Ordering, path::Path};

fn bundle_info(path: &Path) -> Option<Dictionary> {
    let contents = path.join("Contents/Info.plist");
    let path = if contents.exists() {
        contents
    } else {
        path.join("Resources/Info.plist")
    };
    Value::from_file(path).ok()?.into_dictionary()
}
fn is_application(path: &Path, info: &Dictionary) -> bool {
    if !path.is_dir() {
        return false;
    }
    if path.extension().and_then(|s| s.to_str()) == Some("app") {
        return true;
    }
    if path.extension().is_some() {
        return false;
    }
    if info
        .get("CFBundlePackageType")
        .and_then(Value::as_string)
        .is_some_and(|s| s != "APPL")
    {
        return false;
    }
    let executable = info
        .get("CFBundleExecutable")
        .or_else(|| info.get("CFBundleName"))
        .and_then(Value::as_string)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    path.join("Contents/MacOS").join(executable).exists()
}
use autopkg_platform::github::compare_versions as loose_compare;

pub fn create_item(path: &Path) -> Result<Dictionary, String> {
    let mut item = Dictionary::new();
    if let Some(info) = bundle_info(path) {
        item.insert(
            "type".into(),
            Value::String(
                if is_application(path, &info) {
                    "application"
                } else {
                    "bundle"
                }
                .into(),
            ),
        );
        for key in [
            "CFBundleName",
            "CFBundleIdentifier",
            "CFBundleShortVersionString",
            "CFBundleVersion",
        ] {
            if let Some(v) = info.get(key).and_then(Value::as_string) {
                item.insert(key.into(), Value::String(v.into()));
            }
        }
        let minimum = if let Some(v) = info
            .get("LSMinimumSystemVersion")
            .and_then(Value::as_string)
        {
            Some(v.to_owned())
        } else if let Some(versions) = info
            .get("LSMinimumSystemVersionByArchitecture")
            .and_then(Value::as_dictionary)
        {
            versions
                .values()
                .filter_map(Value::as_string)
                .max_by(|a, b| loose_compare(a, b))
                .map(str::to_owned)
        } else {
            info.get("SystemVersionCheck:MinimumSystemVersion")
                .and_then(Value::as_string)
                .map(str::to_owned)
        };
        if let Some(v) = minimum {
            item.insert("minosversion".into(), Value::String(v));
        }
    } else if let Ok(Value::Dictionary(info)) = Value::from_file(path) {
        item.insert("type".into(), Value::String("plist".into()));
        for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
            if let Some(v) = info.get(key).and_then(Value::as_string) {
                item.insert(key.into(), Value::String(v.into()));
            }
        }
    }
    let comparison = match item
        .get("CFBundleShortVersionString")
        .and_then(Value::as_string)
    {
        Some(v) if v.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
            Some("CFBundleShortVersionString")
        }
        _ if item.contains_key("CFBundleVersion") => Some("CFBundleVersion"),
        _ => None,
    };
    if let Some(key) = comparison {
        item.insert("version_comparison_key".into(), Value::String(key.into()));
    }
    if !item.contains_key("CFBundleShortVersionString") && !item.contains_key("CFBundleVersion") {
        item.insert("type".into(), Value::String("file".into()));
        if path.is_file() || path.is_symlink() {
            let hash = std::fs::File::open(path)
                .and_then(|mut file| {
                    let mut hasher = Md5::new();
                    std::io::copy(&mut file, &mut hasher)?;
                    Ok(format!("{:x}", hasher.finalize()))
                })
                .unwrap_or_default();
            item.insert("md5checksum".into(), Value::String(hash));
        }
    }
    item.insert(
        "path".into(),
        Value::String(path.to_string_lossy().into_owned()),
    );
    Ok(item)
}
fn truthy(v: &Value) -> bool {
    crate::truthy(v)
}

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    let paths = env
        .get("installs_item_paths")
        .and_then(Value::as_array)
        .ok_or("installs_item_paths must be an array")?;
    let faux_root = env
        .get("faux_root")
        .and_then(Value::as_string)
        .unwrap_or("")
        .trim_end_matches('/');
    let derive = env.get("derive_minimum_os_version").is_some_and(truthy);
    let mut minimum = env.get("minimum_os_version").cloned();
    let mut items = Vec::new();
    for path in paths {
        let path = path
            .as_string()
            .ok_or("installs_item_paths entries must be strings")?;
        let mut physical = format!("{faux_root}{path}");
        if physical != "/" && physical.ends_with('/') {
            physical.pop();
        }
        if !Path::new(&physical).exists() {
            continue;
        }
        let mut item = create_item(Path::new(&physical))?;
        let logical = physical
            .strip_prefix(faux_root)
            .unwrap_or(&physical)
            .to_owned();
        item.insert("path".into(), Value::String(logical.clone()));
        autopkg_platform::processor_output(1, format!("Created installs item for {logical}"));
        if derive {
            if let Some(v) = item.get("minosversion").and_then(Value::as_string) {
                match minimum.as_ref().and_then(Value::as_string) {
                    None => {
                        minimum = Some(Value::String(v.into()));
                        autopkg_platform::processor_output(1, format!("Derived minimum os version as: {v}"));
                    }
                    Some(old) => match loose_compare(v, old) {
                        Ordering::Greater => {
                            autopkg_platform::processor_output(1, format!("Setting minimum os version to: {v}, as greater than prior value of: {old}"));
                            minimum = Some(Value::String(v.into()));
                        }
                        Ordering::Less => autopkg_platform::processor_output(1, format!("Minimum os version: {v}, is lower than prior value of: {old}... skipping...")),
                        Ordering::Equal => (),
                    },
                }
            }
        }
        let comparison = match env.get("version_comparison_key") {
            Some(Value::String(v)) => Some(v.as_str()),
            Some(Value::Dictionary(v)) => v.get(&logical).and_then(Value::as_string),
            _ => None,
        };
        if let Some(key) = comparison.filter(|s| !s.is_empty()) {
            if !item.contains_key(key) {
                return Err(format!("version_comparison_key '{key}' could not be found in the installs item for path '{logical}'"));
            }
            item.insert("version_comparison_key".into(), Value::String(key.into()));
        }
        items.push(Value::Dictionary(item));
    }
    let mut additional = match env.get("additional_pkginfo") {
        None => Dictionary::new(),
        Some(v) => v
            .as_dictionary()
            .ok_or("additional_pkginfo must be a dictionary")?
            .clone(),
    };
    additional.insert("installs".into(), Value::Array(items));
    if let Some(v) = minimum.filter(truthy) {
        additional.insert("minimum_os_version".into(), v.clone());
        env.insert("minimum_os_version".into(), v);
    }
    env.insert("additional_pkginfo".into(), Value::Dictionary(additional));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(root: &Path, name: &str, short: &str, minimum: &str) -> std::path::PathBuf {
        let path = root.join(name);
        std::fs::create_dir_all(path.join("Contents")).unwrap();
        Value::Dictionary(Dictionary::from_iter([
            ("CFBundleName", "Fixture".into()),
            ("CFBundleIdentifier", "org.fixture".into()),
            ("CFBundleShortVersionString", Value::String(short.into())),
            ("CFBundleVersion", "42".into()),
            ("LSMinimumSystemVersion", Value::String(minimum.into())),
        ]))
        .to_file_xml(path.join("Contents/Info.plist"))
        .unwrap();
        path
    }
    #[test]
    fn app_versions_faux_root_and_minimum_os() {
        let temp = tempfile::tempdir().unwrap();
        app(temp.path(), "Applications/Test.app", "2.0", "13.4");
        app(temp.path(), "Library/Test.bundle", "invalid", "14.1");
        let mut env = Dictionary::from_iter([
            (
                "installs_item_paths",
                Value::Array(vec![
                    "/Applications/Test.app".into(),
                    "/Library/Test.bundle".into(),
                    "/missing".into(),
                ]),
            ),
            (
                "faux_root",
                Value::String(temp.path().to_string_lossy().into_owned()),
            ),
            ("derive_minimum_os_version", true.into()),
        ]);
        execute(&mut env).unwrap();
        let info = env["additional_pkginfo"].as_dictionary().unwrap();
        let items = info["installs"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        let first = items[0].as_dictionary().unwrap();
        let second = items[1].as_dictionary().unwrap();
        assert_eq!(first["type"].as_string(), Some("application"));
        assert_eq!(first["path"].as_string(), Some("/Applications/Test.app"));
        assert_eq!(second["type"].as_string(), Some("bundle"));
        assert_eq!(
            second["version_comparison_key"].as_string(),
            Some("CFBundleVersion")
        );
        assert_eq!(info["minimum_os_version"].as_string(), Some("14.1"));
    }
    #[test]
    fn files_plists_and_comparison_override_failure() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, "hello").unwrap();
        assert_eq!(
            create_item(&file).unwrap()["md5checksum"].as_string(),
            Some("5d41402abc4b2a76b9719d911017c592")
        );
        assert!(!create_item(temp.path())
            .unwrap()
            .contains_key("md5checksum"));
        let plist = temp.path().join("info.plist");
        Value::Dictionary(Dictionary::from_iter([(
            "CFBundleVersion",
            Value::String("9".into()),
        )]))
        .to_file_xml(&plist)
        .unwrap();
        let info = create_item(&plist).unwrap();
        assert_eq!(info["type"].as_string(), Some("plist"));
        assert_eq!(
            info["version_comparison_key"].as_string(),
            Some("CFBundleVersion")
        );
        let mut env = Dictionary::from_iter([
            (
                "installs_item_paths",
                Value::Array(vec![file.to_string_lossy().into_owned().into()]),
            ),
            ("version_comparison_key", "CFBundleVersion".into()),
        ]);
        assert!(execute(&mut env)
            .unwrap_err()
            .contains("could not be found"));
        assert!(!env.contains_key("additional_pkginfo"));
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires the pinned Munki 7.2.0.5787 development reference"]
    fn differential_pinned_makepkginfo() {
        use std::process::Command;
        let binary = "/usr/local/munki/makepkginfo";
        let version = Command::new(binary).arg("--version").output().unwrap();
        assert!(version.status.success());
        assert_eq!(
            String::from_utf8_lossy(&version.stdout).trim(),
            crate::REFERENCE_VERSION
        );
        let temp = tempfile::tempdir().unwrap();
        let application = app(temp.path(), "Test.app", "2.0", "13.4");
        let bundle = app(temp.path(), "Plugin.bundle", "invalid", "14.1");
        let plain = temp.path().join("é-file");
        std::fs::write(&plain, "hello λ").unwrap();
        let directory = temp.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        let plist = temp.path().join("info.plist");
        Value::Dictionary(Dictionary::from_iter([(
            "CFBundleVersion",
            Value::String("9".into()),
        )]))
        .to_file_xml(&plist)
        .unwrap();
        let no_version = temp.path().join("no-version.plist");
        Value::Dictionary(Dictionary::from_iter([(
            "name",
            Value::String("test".into()),
        )]))
        .to_file_xml(&no_version)
        .unwrap();
        let paths = [application, bundle, plain, directory, plist, no_version];
        let mut command = Command::new(binary);
        for path in &paths {
            command.arg("-f").arg(path);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = Value::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
        let mut env = Dictionary::from_iter([(
            "installs_item_paths",
            Value::Array(
                paths
                    .iter()
                    .map(|p| Value::String(p.to_string_lossy().into_owned()))
                    .collect(),
            ),
        )]);
        execute(&mut env).unwrap();
        assert_eq!(
            env["additional_pkginfo"].as_dictionary().unwrap()["installs"],
            expected.as_dictionary().unwrap()["installs"]
        );
    }
}
