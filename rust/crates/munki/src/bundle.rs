//! Legacy bundle-package metadata. Payloads are inspected, never installed.
use crate::metadata::Options;
use autopkg_platform::github::compare_versions;
use plist::{Dictionary, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    path::{Path, PathBuf},
};

fn bundle_info(path: &Path) -> Option<Dictionary> {
    let contents = path.join("Contents/Info.plist");
    let path = if contents.exists() {
        contents
    } else {
        path.join("Resources/Info.plist")
    };
    Value::from_file(path).ok()?.into_dictionary()
}
fn old_info(path: &Path) -> Dictionary {
    let directory = path.join("Contents/Resources/English.lproj");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Dictionary::new();
    };
    for entry in entries.filter_map(Result::ok) {
        if entry.path().extension().and_then(|s| s.to_str()) != Some("info") {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path()) {
            let (text, _, _) = encoding_rs::MACINTOSH.decode(&bytes);
            let mut info = Dictionary::new();
            for line in text.lines() {
                let parts: Vec<_> = line.split(|c: char| c.is_whitespace()).collect();
                if parts.len() > 1 {
                    info.insert(parts[0].into(), Value::String(parts[1..].join(" ")));
                }
            }
            return info;
        }
    }
    Dictionary::new()
}
fn version(path: &Path) -> String {
    if let Some(info) = bundle_info(path) {
        for key in ["CFBundleShortVersionString", "CFBundleVersion"] {
            if let Some(v) = info
                .get(key)
                .and_then(Value::as_string)
                .filter(|s| !s.is_empty())
            {
                return v.into();
            }
        }
    }
    old_info(path)
        .get("Version")
        .and_then(Value::as_string)
        .unwrap_or("")
        .into()
}
fn receipt(path: &Path) -> Option<Dictionary> {
    let name = path.file_name()?.to_string_lossy();
    if let Some(info) = bundle_info(path) {
        let id = info
            .get("CFBundleIdentifier")
            .or_else(|| info.get("Bundle identifier"))
            .and_then(Value::as_string)
            .unwrap_or(&name)
            .to_owned();
        let mut receipt = Dictionary::from_iter([
            ("filename", Value::String(name.into_owned())),
            ("packageid", Value::String(id)),
            ("version", Value::String(version(path))),
        ]);
        if let Some(v) = info.get("CFBundleName").and_then(Value::as_string) {
            receipt.insert("name".into(), Value::String(v.into()));
        }
        if let Some(Value::Integer(v)) = info.get("IFPkgFlagInstalledSize") {
            receipt.insert("installed_size".into(), Value::Integer(*v));
        }
        Some(receipt)
    } else {
        let info = old_info(path);
        if info.is_empty() {
            return None;
        }
        Some(Dictionary::from_iter([
            ("filename", Value::String(name.to_string())),
            ("packageid", Value::String(name.to_string())),
            (
                "name",
                info.get("Title")
                    .cloned()
                    .unwrap_or(Value::String(name.into_owned())),
            ),
            (
                "version",
                info.get("Version")
                    .cloned()
                    .unwrap_or(Value::String("0.0".into())),
            ),
        ]))
    }
}
fn dist_receipts(path: &Path) -> Result<Vec<Value>, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let document = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
    let mut items: BTreeMap<String, Dictionary> = BTreeMap::new();
    let mut files = BTreeSet::new();
    for node in document.descendants().filter(|n| n.has_tag_name("pkg-ref")) {
        let Some(id) = node.attribute("id") else {
            continue;
        };
        let item = items
            .entry(id.into())
            .or_insert_with(|| Dictionary::from_iter([("packageid", Value::String(id.into()))]));
        if let Some(v) = node.attribute("version") {
            item.insert("version".into(), Value::String(v.into()));
        }
        if let Some(size) = node
            .attribute("installKBytes")
            .and_then(|s| s.parse::<u64>().ok())
        {
            item.insert("installed_size".into(), Value::Integer(size.into()));
        }
        if node.text().is_some_and(|s| !s.is_empty()) {
            files.insert(id.to_owned());
        }
    }
    Ok(items
        .into_iter()
        .filter(|(id, item)| files.contains(id) && item.contains_key("version"))
        .map(|(_, item)| Value::Dictionary(item))
        .collect())
}
fn receipts(path: &Path, active: &mut HashSet<PathBuf>) -> Result<Vec<Value>, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if !active.insert(canonical.clone()) {
        return Err("Bundle package component cycle".into());
    }
    let result = (|| {
        if path.extension().and_then(|s| s.to_str()) == Some("pkg") {
            if let Some(receipt) = receipt(path) {
                return Ok(vec![Value::Dictionary(receipt)]);
            }
        }
        let contents = path.join("Contents");
        if !contents.is_dir() {
            return Ok(Vec::new());
        }
        for entry in std::fs::read_dir(&contents)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
        {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("dist") {
                return dist_receipts(&entry.path());
            }
        }
        let component_dir = bundle_info(path).and_then(|info| {
            info.get("IFPkgFlagComponentDirectory")
                .and_then(Value::as_string)
                .map(str::to_owned)
        });
        let directories = component_dir.map(|d| vec![d]).unwrap_or_else(|| {
            [
                "",
                "Contents",
                "Contents/Installers",
                "Contents/Packages",
                "Contents/Resources",
                "Contents/Resources/Packages",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect()
        });
        let mut result = Vec::new();
        for directory in directories {
            let directory = path.join(directory);
            if !directory.is_dir() {
                continue;
            }
            if !directory
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&canonical)
            {
                return Err("Bundle component directory resolves outside package".into());
            }
            for entry in std::fs::read_dir(directory)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
            {
                let child = entry.path();
                if !child.is_dir() {
                    continue;
                }
                match child.extension().and_then(|s| s.to_str()) {
                    Some("pkg") => {
                        if let Some(receipt) = receipt(&child) {
                            result.push(Value::Dictionary(receipt));
                        }
                    }
                    Some("mpkg") => result.extend(receipts(&child, active)?),
                    _ => {}
                }
            }
        }
        Ok(result)
    })();
    active.remove(&canonical);
    result
}
pub(crate) fn package(path: &Path, options: &Options) -> Result<Dictionary, String> {
    let receipts = receipts(path, &mut HashSet::new())?;
    let mut version = version(path);
    if version.is_empty() {
        version = receipts
            .iter()
            .filter_map(Value::as_dictionary)
            .filter_map(|d| d.get("version").and_then(Value::as_string))
            .max_by(|a, b| compare_versions(a, b))
            .unwrap_or("")
            .into();
    }
    if version.is_empty() {
        version = "0.0.0.0.0".into();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut name = stem.to_string();
    for delimiter in ["--", "-"] {
        if let Some((n, v)) = stem.rsplit_once(delimiter) {
            if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                name = n.into();
                break;
            }
        }
    }
    let size: i64 = receipts
        .iter()
        .filter_map(Value::as_dictionary)
        .filter_map(|d| d.get("installed_size").and_then(Value::as_signed_integer))
        .sum();
    let mut info = Dictionary::from_iter([
        ("name", Value::String(name)),
        ("version", Value::String(version)),
    ]);
    if !receipts.is_empty() {
        info.insert("receipts".into(), Value::Array(receipts));
    }
    if size > 0 {
        info.insert("installed_size".into(), Value::Integer(size.into()));
    }
    let flag = bundle_info(path).and_then(|d| {
        d.get("IFPkgFlagRestartAction")
            .and_then(Value::as_string)
            .map(str::to_owned)
    });
    if let Some(action) = crate::tools::restart_action(path, Some(flag.as_deref().unwrap_or("")))? {
        info.insert("RestartAction".into(), Value::String(action));
    }
    if options.flag("installerChoices") {
        if let Some(choices) = crate::tools::installer_choices(path)? {
            info.insert("installer_choices_xml".into(), Value::Array(choices));
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundle_receipt_and_component_versions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Fixture.pkg");
        std::fs::create_dir_all(path.join("Contents")).unwrap();
        Value::Dictionary(Dictionary::from_iter([
            ("CFBundleIdentifier", Value::String("org.test".into())),
            ("CFBundleShortVersionString", "2.0".into()),
            ("CFBundleName", "Fixture".into()),
            ("IFPkgFlagInstalledSize", Value::Integer(42.into())),
        ]))
        .to_file_xml(path.join("Contents/Info.plist"))
        .unwrap();
        let items = receipts(&path, &mut HashSet::new()).unwrap();
        let item = items[0].as_dictionary().unwrap();
        assert_eq!(item["filename"].as_string(), Some("Fixture.pkg"));
        assert_eq!(item["version"].as_string(), Some("2.0"));
        assert_eq!(item["installed_size"].as_signed_integer(), Some(42));
    }
}
