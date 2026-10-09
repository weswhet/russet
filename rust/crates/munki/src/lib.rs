//! The metadata compatibility target for the native FileRepo implementation.
pub const REFERENCE_VERSION: &str = "7.2.0.5787";
mod bundle;
pub mod catalog;
#[cfg(unix)]
mod icon_native;
pub mod icons;
pub mod metadata;
mod mount;
mod osinstaller;
pub mod processors;
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
        "MunkiImporter" => processors::munki_importer::execute(env),
        "MunkiInfoCreator" => processors::munki_info_creator::execute(env),
        "MunkiInstallsItemsCreator" => processors::munki_installs_items_creator::execute(env),
        "MunkiOptionalReceiptEditor" => processors::munki_optional_receipt_editor::execute(env),
        "MunkiPkginfoMerger" => processors::munki_pkginfo_merger::execute(env),
        "MunkiSetDefaultCatalog" => processors::munki_set_default_catalog::execute(env),
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
