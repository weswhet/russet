//! `MakeCatalogsProcessor`: rebuild a Munki repo's catalogs. A native port
//! of the autopkg/recipes processor (Apache-2.0) that follows Munki 7.2.0's
//! native makecatalogs.
use crate::community_builders::{output, string, truthy};
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

fn repo_files(root: &Path) -> Vec<PathBuf> {
    fn visit(root: &Path, ancestors: &mut Vec<PathBuf>, files: &mut Vec<PathBuf>) {
        let Ok(real) = fs::canonicalize(root) else {
            return;
        };
        if ancestors.contains(&real) {
            return;
        }
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        ancestors.push(real);
        let mut dirs = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.path().is_dir() {
                dirs.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
        for dir in dirs {
            visit(&dir, ancestors, files);
        }
        ancestors.pop();
    }
    let mut files = Vec::new();
    visit(root, &mut Vec::new(), &mut files);
    files
}
fn repo_path(value: &str) -> Result<PathBuf, String> {
    if value.starts_with("file:") {
        url::Url::parse(value)
            .map_err(|e| e.to_string())?
            .to_file_path()
            .map_err(|_| "Invalid FileRepo URL".into())
    } else if Path::new(value).is_absolute() {
        Ok(value.into())
    } else {
        Err("MakeCatalogsProcessor requires a local FileRepo path or file URL".into())
    }
}
fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub(crate) fn execute(
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
) -> Result<(), String> {
    autopkg_munki::validate_backend(env)?;
    let cache = preferences
        .and_then(|p| p.get("CACHE_DIR"))
        .and_then(Value::as_string)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/AutoPkg/Cache")
        });
    let changed = match fs::read(cache.join("autopkg_results.plist")) {
        Ok(bytes) => {
            let results = Value::from_reader(std::io::Cursor::new(bytes))
                .map_err(|e| format!("Invalid autopkg_results.plist: {e}"))?;
            results
                .as_array()
                .ok_or("autopkg_results.plist must contain an array")?
                .iter()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(Value::as_dictionary)
                .filter_map(|d| d.get("Output"))
                .filter_map(Value::as_dictionary)
                .any(|d| d.get("munki_repo_changed").is_some_and(truthy))
        }
        Err(_) => false,
    };
    if !changed && !env.get("force_rebuild").is_some_and(truthy) {
        output("No need to rebuild catalogs.");
        env.insert("makecatalogs_resultcode".into(), 0.into());
        env.insert("makecatalogs_stderr".into(), "".into());
        return Ok(());
    }
    let root = repo_path(string(env, "MUNKI_REPO")?)?;
    let result = rebuild_catalogs(&root);
    let (warnings, errors) = match result {
        Ok(v) => v,
        Err(e) => (Vec::new(), vec![e]),
    };
    let stderr = warnings
        .iter()
        .chain(&errors)
        .map(|s| format!("{s}\n"))
        .collect::<String>();
    env.insert(
        "makecatalogs_resultcode".into(),
        if errors.is_empty() {
            0.into()
        } else {
            1.into()
        },
    );
    env.insert("makecatalogs_stderr".into(), stderr.clone().into());
    if errors.is_empty() {
        output("Munki catalogs rebuilt!");
        Ok(())
    } else {
        Err(format!("makecatalogs failed: \n{stderr}"))
    }
}
pub(crate) fn verify_payload(
    info: &Dictionary,
    identifier: &str,
    packages: &[String],
    warnings: &mut Vec<String>,
) -> bool {
    if matches!(
        info.get("installer_type").and_then(Value::as_string),
        Some("nopkg" | "apple_update_metadata")
    ) || ["PackageCompleteURL", "PackageURL"].iter().any(|k| {
        info.get(k)
            .and_then(Value::as_string)
            .is_some_and(|s| !s.is_empty())
    }) {
        return true;
    }
    for (key, description) in [
        ("installer_item_location", "installer"),
        ("uninstaller_item_location", "uninstaller"),
    ] {
        if key.starts_with("uninstaller") && info.get(key).and_then(Value::as_string).is_none() {
            continue;
        }
        let location = info.get(key).and_then(Value::as_string).unwrap_or("");
        if location.is_empty() {
            warnings.push(format!("WARNING: empty or invalid {key} in {identifier}"));
            return false;
        }
        if !packages.iter().any(|p| p == location) {
            if let Some(found) = packages
                .iter()
                .find(|p| p.to_lowercase() == location.to_lowercase())
            {
                warnings.push(format!("WARNING: {identifier} refers to {description} item: {location}. The pathname of the item in the repo has different case: {found}. This may cause issues depending on the case-sensitivity of the underlying filesystem."));
            } else {
                warnings.push(format!(
                    "WARNING: {identifier} refers to missing {description} item: {location}"
                ));
                return false;
            }
        }
    }
    true
}
pub(crate) fn rebuild_catalogs(root: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    if !root.is_dir() {
        return Err(format!("Repo error: {} is not a directory", root.display()));
    }
    let info_paths = repo_files(&root.join("pkgsinfo"));
    let packages = repo_files(&root.join("pkgs"))
        .iter()
        .map(|p| {
            p.strip_prefix(root.join("pkgs"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    let (mut warnings, mut errors) = (Vec::new(), Vec::new());
    let mut catalogs: BTreeMap<String, Vec<Value>> = BTreeMap::from([("all".into(), Vec::new())]);
    for path in info_paths {
        let identifier = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut info = match Value::from_file(&path) {
            Ok(value) => value.into_dictionary().unwrap_or_default(),
            Err(e) => {
                errors.push(format!("Unexpected error reading {identifier}: {e}"));
                continue;
            }
        };
        if !info.contains_key("name") {
            warnings.push(format!("WARNING: {identifier} is missing name key"));
            continue;
        }
        info.retain(|key, _| key != "notes" && !key.starts_with('_'));
        if !verify_payload(&info, &identifier, &packages, &mut warnings) {
            continue;
        }
        let value = Value::Dictionary(info.clone());
        catalogs.get_mut("all").unwrap().push(value.clone());
        let names = info
            .get("catalogs")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(Value::as_string).collect::<Option<Vec<_>>>());
        match names {
            Some(names) if names.is_empty() => warnings.push(format!(
                "WARNING: {identifier} has an empty catalogs array!"
            )),
            Some(names) => {
                for name in names {
                    if !safe_relative(name) {
                        errors.push(format!("Invalid catalog name: {name}"));
                        continue;
                    }
                    catalogs.entry(name.into()).or_default().push(value.clone());
                }
            }
            None => warnings.push(format!("WARNING: {identifier} has no catalogs array!")),
        }
    }
    let duplicates: Vec<_> = catalogs
        .keys()
        .filter(|name| {
            catalogs
                .keys()
                .any(|other| other != *name && other.to_lowercase() == name.to_lowercase())
        })
        .collect();
    if !duplicates.is_empty() {
        warnings.push(format!("WARNING: There are catalogs with names that differ only by case. This may cause issues depending on the case-sensitivity of the underlying filesystem: {duplicates:?}"));
    }
    for old in repo_files(&root.join("catalogs")) {
        let name = old
            .strip_prefix(root.join("catalogs"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !catalogs.contains_key(&name) {
            if let Err(e) = fs::remove_file(&old) {
                errors.push(format!("Could not delete catalog {name}: {e}"));
            }
        }
    }
    for (name, items) in catalogs {
        if items.is_empty() {
            continue;
        } // Munki 7 retains an existing empty catalog.
        let path = root.join("catalogs").join(&name);
        let result = fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| e.to_string())
            .and_then(|_| {
                Value::Array(items)
                    .to_file_xml(&path)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            errors.push(format!("Failed to create catalog {name}: {e}"));
        }
    }
    let mut hashes = Dictionary::new();
    {
        let icons = repo_files(&root.join("icons"));
        for path in icons {
            let name = path
                .strip_prefix(root.join("icons"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if name == "_icon_hashes.plist" {
                continue;
            }
            match fs::read(&path) {
                Ok(bytes) => {
                    hashes.insert(name, format!("{:x}", Sha256::digest(bytes)).into());
                }
                Err(e) => errors.push(format!("Error reading icons/{name}: {e}")),
            }
        }
    }
    if !hashes.is_empty() {
        if let Err(e) = Value::Dictionary(hashes).to_file_xml(root.join("icons/_icon_hashes.plist"))
        {
            errors.push(format!("Failed to create icons/_icon_hashes.plist: {e}"));
        }
    }
    Ok((warnings, errors))
}
