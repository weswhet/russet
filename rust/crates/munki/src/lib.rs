//! The metadata compatibility target for the native FileRepo implementation.
pub const REFERENCE_VERSION: &str = "7.2.0.5787";
mod bundle;
pub mod catalog;
pub mod icons;
pub mod importer;
pub mod installs;
pub mod metadata;
mod mount;
mod osinstaller;
mod tools;

use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

fn string<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    env.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("Missing or invalid string input: {key}"))
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Boolean(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Integer(v) => v.as_signed() != Some(0),
        Value::Real(v) => *v != 0.0,
        Value::Array(v) => !v.is_empty(),
        Value::Dictionary(v) => !v.is_empty(),
        Value::Data(v) => !v.is_empty(),
        _ => true,
    }
}

pub fn validate_backend(env: &Dictionary) -> Result<(), String> {
    if env
        .get("MUNKI_REPO_PLUGIN")
        .is_some_and(|v| v.as_string() != Some("FileRepo"))
    {
        return Err("Only the Munki FileRepo backend is supported".into());
    }
    if env.get("force_munki_repo_lib").is_some_and(truthy) {
        return Err("Python Munki repository libraries are unsupported".into());
    }
    Ok(())
}

/// Local repository operations shared by metadata processors.
pub struct FileRepo {
    root: PathBuf,
}
impl FileRepo {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn catalog(&self) -> Result<Vec<Value>, String> {
        let path = self.root.join("catalogs/all");
        if !path.exists() {
            return Ok(Vec::new());
        }
        Value::from_file(&path)
            .map_err(|e| format!("Error reading 'all' catalog: {e}"))?
            .into_array()
            .ok_or_else(|| "Munki 'all' catalog must be an array".into())
    }
    pub fn index(&self) -> Result<catalog::CatalogIndex, String> {
        catalog::CatalogIndex::new(self.catalog()?)
    }

    pub fn put_pkginfo(&self, info: &Dictionary, path: &Path) -> Result<(), String> {
        Value::Dictionary(info.clone())
            .to_file_xml(path)
            .map_err(|e| format!("Could not write pkginfo {}: {e}", path.display()))
    }
}

pub fn execute(name: &str, env: &mut Dictionary) -> Result<(), String> {
    validate_backend(env)?;
    match name {
        "MunkiImporter" => importer::execute(env),
        "MunkiInfoCreator" => {
            let mut arguments = Vec::new();
            for key in ["displayname", "description", "catalog"] {
                if let Some(v) = env.get(key).and_then(Value::as_string) {
                    arguments.push(format!("--{key}={v}"));
                }
            }
            let options = metadata::Options::parse(&arguments)?;
            let mut info = metadata::generate(Some(Path::new(string(env, "pkg_path")?)), &options)?;
            for key in ["name", "version"] {
                if let Some(v) = env.get(key) {
                    info.insert(key.into(), v.clone());
                }
            }
            if let Some(path) = env.get("info_path").and_then(Value::as_string) {
                Value::Dictionary(info.clone())
                    .to_file_xml(path)
                    .map_err(|e| e.to_string())?;
            }
            env.insert("munki_info".into(), Value::Dictionary(info));
            Ok(())
        }
        "MunkiInstallsItemsCreator" => installs::execute(env),
        "MunkiSetDefaultCatalog" => {
            if !env.contains_key("pkginfo") {
                env.insert("pkginfo".into(), Value::Dictionary(Dictionary::new()));
            }
            let mut changed = false;
            if let Some(catalog) =
                autopkg_platform::preference("com.googlecode.munki.munkiimport", "default_catalog")?
            {
                let nonempty = truthy(&catalog);
                if nonempty {
                    let message = format!(
                        "Updated target catalogs into pkginfo with {}",
                        plist::python_str(&catalog)
                    );
                    env.get_mut("pkginfo")
                        .and_then(Value::as_dictionary_mut)
                        .ok_or("pkginfo must be a dictionary")?
                        .insert("catalogs".into(), Value::Array(vec![catalog]));
                    changed = true;
                    autopkg_platform::processor_output(1, message);
                }
            }
            if !changed {
                autopkg_platform::processor_output(1, "No default catalogs found, nothing changed");
            }
            Ok(())
        }
        "MunkiPkginfoMerger" => {
            let additional = env
                .get("additional_pkginfo")
                .and_then(Value::as_dictionary)
                .ok_or("additional_pkginfo must be a dictionary")?
                .clone();
            if !env.contains_key("pkginfo") {
                env.insert("pkginfo".into(), Value::Dictionary(Dictionary::new()));
            }
            let info = env
                .get_mut("pkginfo")
                .and_then(Value::as_dictionary_mut)
                .ok_or("pkginfo must be a dictionary")?;
            let message = format!(
                "Merged {} into pkginfo",
                plist::python_repr(&Value::Dictionary(additional.clone()))
            );
            info.extend(additional);
            autopkg_platform::processor_output(1, message);
            Ok(())
        }
        "MunkiOptionalReceiptEditor" => {
            let path = string(env, "pkginfo_repo_path")?.to_owned();
            if path.is_empty() {
                autopkg_platform::processor_output(1, "No pkginfo_repo_path specified, skipping");
                if !env.contains_key("munki_info") {
                    env.insert("munki_info".into(), Value::Dictionary(Dictionary::new()));
                }
                return Ok(());
            }
            let ids = env
                .get("pkg_ids_set_optional_true")
                .and_then(Value::as_array)
                .ok_or("pkg_ids_set_optional_true must be an array")?;
            let mut info = match env
                .get("munki_info")
                .and_then(Value::as_dictionary)
                .filter(|d| !d.is_empty())
            {
                Some(info) => info.clone(),
                None => Value::from_file(&path)
                    .map_err(|e| e.to_string())?
                    .into_dictionary()
                    .ok_or("pkginfo must be a dictionary")?,
            };
            let receipts = info
                .get_mut("receipts")
                .and_then(Value::as_array_mut)
                .ok_or("pkginfo does not contain any receipts")?;
            let mut changed = false;
            for receipt in receipts {
                let receipt = receipt
                    .as_dictionary_mut()
                    .ok_or("Invalid receipt dictionary")?;
                let id = receipt
                    .get("packageid")
                    .ok_or("Receipt does not contain packageid")?;
                if ids.contains(id) {
                    autopkg_platform::processor_output(
                        1,
                        format!("Setting package ID {} as optional", plist::python_str(id)),
                    );
                    receipt.insert("optional".into(), Value::Boolean(true));
                    changed = true;
                }
            }
            if changed {
                autopkg_platform::processor_output(1, format!("Writing pkginfo to {path}"));
                FileRepo::new(string(env, "MUNKI_REPO")?).put_pkginfo(&info, Path::new(&path))?;
            } else {
                autopkg_platform::processor_output(1, "No receipts modified, nothing to do");
            }
            env.insert("munki_info".into(), Value::Dictionary(info));
            Ok(())
        }
        _ => Err(format!("Munki processor '{name}' is not implemented")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn null_and_empty_optional_flags_are_false() {
        assert!(validate_backend(&Dictionary::from_iter([(
            "force_munki_repo_lib",
            Value::Null
        )]))
        .is_ok());
        for value in [
            Value::Null,
            Value::Real(0.0),
            Value::Array(vec![]),
            Value::Dictionary(Dictionary::new()),
            Value::Data(vec![]),
        ] {
            assert!(!truthy(&value));
        }
    }
    #[test]
    fn merger_overwrites_shallowly_and_preserves_types() {
        let mut env = Dictionary::from_iter([
            (
                "pkginfo",
                Value::Dictionary(Dictionary::from_iter([
                    ("name", Value::String("app".into())),
                    ("managed", false.into()),
                ])),
            ),
            (
                "additional_pkginfo",
                Value::Dictionary(Dictionary::from_iter([
                    ("managed", true.into()),
                    ("data", Value::Data(vec![0, 255])),
                ])),
            ),
        ]);
        execute("MunkiPkginfoMerger", &mut env).unwrap();
        let info = env["pkginfo"].as_dictionary().unwrap();
        assert_eq!(info["name"].as_string(), Some("app"));
        assert_eq!(info["managed"].as_boolean(), Some(true));
        assert_eq!(info["data"].as_data(), Some(&[0, 255][..]));
    }

    #[test]
    fn receipt_edit_is_repeatable_and_preserves_unselected_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("info.plist");
        let receipt = |id: &str| {
            Value::Dictionary(Dictionary::from_iter([
                ("packageid", Value::String(id.into())),
                ("version", "1".into()),
            ]))
        };
        let info =
            Dictionary::from_iter([("receipts", Value::Array(vec![receipt("a"), receipt("b")]))]);
        Value::Dictionary(info).to_file_xml(&path).unwrap();
        let mut env = Dictionary::from_iter([
            (
                "MUNKI_REPO",
                dir.path().to_string_lossy().into_owned().into(),
            ),
            (
                "pkginfo_repo_path",
                path.to_string_lossy().into_owned().into(),
            ),
            ("pkg_ids_set_optional_true", Value::Array(vec!["a".into()])),
        ]);
        execute("MunkiOptionalReceiptEditor", &mut env).unwrap();
        let first = std::fs::read(&path).unwrap();
        execute("MunkiOptionalReceiptEditor", &mut env).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), first);
        let result = Value::from_file(&path).unwrap();
        let receipts = result.as_dictionary().unwrap()["receipts"]
            .as_array()
            .unwrap();
        assert_eq!(
            receipts[0].as_dictionary().unwrap()["optional"].as_boolean(),
            Some(true)
        );
        assert!(!receipts[1]
            .as_dictionary()
            .unwrap()
            .contains_key("optional"));
    }

    #[test]
    fn rejects_python_backends_before_mutation() {
        let mut env = Dictionary::from_iter([("force_munki_repo_lib", Value::Boolean(true))]);
        assert!(execute("MunkiPkginfoMerger", &mut env).is_err());
        assert!(!env.contains_key("pkginfo"));
    }
}
