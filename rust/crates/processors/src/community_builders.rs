//! Native ports of the AutoPkg recipe builders (Apache-2.0).
//! Framework relocation follows gregneagle/relocatable-python at 8ee72fe3;
//! catalog generation follows Munki 7.2.0's native makecatalogs.
use plist::{Dictionary, Value};

pub(crate) fn output(message: impl AsRef<str>) {
    autopkg_platform::processor_output(1, message.as_ref());
}

pub(crate) fn string<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    env.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("Missing or invalid string input: {key}"))
}
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Boolean(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Integer(v) => v.as_signed() != Some(0),
        Value::Real(v) => *v != 0.,
        Value::Array(v) => !v.is_empty(),
        Value::Dictionary(v) => !v.is_empty(),
        Value::Data(v) => !v.is_empty(),
        _ => true,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::processors::{
        autopkg_source_finder::execute as source_finder,
        generate_relocatable_python::{
            execute as generate_python, fix_scripts, install_sitecustomize, relative_path,
            SITECUSTOMIZE,
        },
        make_catalogs_processor::{execute as make_catalogs, rebuild_catalogs, verify_payload},
    };
    use sha2::{Digest, Sha256};
    use std::{fs, path::Path};
    #[test]
    fn source_finder_matches_and_preserves_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([(
            "input_path",
            Value::String(dir.path().display().to_string()),
        )]);
        source_finder(&mut env).unwrap();
        assert_eq!(
            env["autopkg_path"].as_string().unwrap(),
            format!("{}/", dir.path().display())
        );
        fs::create_dir(dir.path().join("autopkg-autopkg-abc")).unwrap();
        source_finder(&mut env).unwrap();
        assert!(env["autopkg_path"]
            .as_string()
            .unwrap()
            .ends_with("autopkg-autopkg-abc"));
    }
    #[test]
    fn catalogs_strip_private_data_hash_icons_and_remove_stale_catalogs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in ["pkgs", "pkgsinfo", "catalogs", "icons"] {
            fs::create_dir(root.join(folder)).unwrap();
        }
        let info = Dictionary::from_iter([
            ("name", "Example".into()),
            ("version", "1".into()),
            ("installer_type", "nopkg".into()),
            ("notes", "private".into()),
            ("_metadata", "private".into()),
            ("catalogs", Value::Array(vec!["testing".into()])),
        ]);
        Value::Dictionary(info.clone())
            .to_file_xml(root.join("pkgsinfo/item"))
            .unwrap();
        fs::write(root.join("icons/example.png"), b"icon").unwrap();
        fs::write(root.join("catalogs/obsolete"), b"stale").unwrap();
        let (warnings, errors) = rebuild_catalogs(root).unwrap();
        assert!(warnings.is_empty());
        assert!(errors.is_empty());
        assert!(!root.join("catalogs/obsolete").exists());
        let all = Value::from_file(root.join("catalogs/all")).unwrap();
        let result = all.as_array().unwrap()[0].as_dictionary().unwrap();
        assert!(!result.contains_key("notes"));
        assert!(!result.contains_key("_metadata"));
        assert_eq!(
            Value::from_file(root.join("pkgsinfo/item")).unwrap(),
            Value::Dictionary(info)
        );
        let hashes = Value::from_file(root.join("icons/_icon_hashes.plist")).unwrap();
        assert_eq!(
            hashes.as_dictionary().unwrap()["example.png"]
                .as_string()
                .unwrap(),
            format!("{:x}", Sha256::digest(b"icon"))
        );
        assert_eq!(
            Value::from_file(root.join("catalogs/testing")).unwrap(),
            all
        );
    }
    #[test]
    fn cached_results_trigger_rebuild_and_false_flag_skips() {
        let dir = tempfile::tempdir().unwrap();
        let prefs =
            Dictionary::from_iter([("CACHE_DIR", Value::String(dir.path().display().to_string()))]);
        let mut env = Dictionary::from_iter([(
            "MUNKI_REPO",
            Value::String(
                dir.path()
                    .join("missing-test-repository")
                    .display()
                    .to_string(),
            ),
        )]);
        make_catalogs(&mut env, Some(&prefs)).unwrap();
        assert_eq!(env["makecatalogs_resultcode"].as_signed_integer(), Some(0));
        let output = Value::Dictionary(Dictionary::from_iter([(
            "munki_repo_changed",
            Value::Boolean(true),
        )]));
        let item = Value::Dictionary(Dictionary::from_iter([("Output", output)]));
        Value::Array(vec![Value::Array(vec![item])])
            .to_file_xml(dir.path().join("autopkg_results.plist"))
            .unwrap();
        assert!(make_catalogs(&mut env, Some(&prefs)).is_err());
        assert_eq!(env["makecatalogs_resultcode"].as_signed_integer(), Some(1));
    }
    #[test]
    fn verifies_missing_and_case_mismatched_payloads() {
        let info =
            Dictionary::from_iter([("installer_item_location", Value::String("item.pkg".into()))]);
        let mut warnings = Vec::new();
        assert!(!verify_payload(
            &info,
            "pkgsinfo/example",
            &[],
            &mut warnings
        ));
        assert!(warnings[0].contains("missing installer"));
        warnings.clear();
        assert!(verify_payload(
            &info,
            "pkgsinfo/example",
            &["Item.pkg".into()],
            &mut warnings
        ));
        assert!(warnings[0].contains("different case"));
    }
    #[test]
    fn sitecustomize_is_preserved_and_relative_paths_work() {
        let dir = tempfile::tempdir().unwrap();
        install_sitecustomize(dir.path(), "3.13").unwrap();
        let path = dir
            .path()
            .join("Versions/3.13/lib/python3.13/site-packages/sitecustomize.py");
        assert_eq!(fs::read_to_string(&path).unwrap(), SITECUSTOMIZE);
        fs::write(&path, "existing").unwrap();
        install_sitecustomize(dir.path(), "3.13").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "existing");
        assert_eq!(
            relative_path(
                Path::new("/a/Python.framework"),
                Path::new("/a/Python.framework/Versions/3.13/bin")
            ),
            Path::new("../../..")
        );
    }
    #[cfg(unix)]
    #[test]
    fn relocates_script_shebang_and_retains_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Python.framework");
        let bin = root.join("Versions/3.13/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("python3.13"), "binary placeholder").unwrap();
        let script = bin.join("pip3");
        fs::write(
            &script,
            "#!/Library/Frameworks/Python.framework/Versions/3.13/bin/python3.13\nprint('hello')\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fix_scripts(&root, "3.13").unwrap();
        let text = fs::read_to_string(&script).unwrap();
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("/python3.13"));
        assert!(text.ends_with("print('hello')\n"));
        assert_eq!(
            fs::metadata(script).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_link(root.join("Versions/Current")).unwrap(),
            Path::new("3.13")
        );
    }
    #[test]
    fn catalog_hidden_files_and_missing_resource_directories_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pkgsinfo")).unwrap();
        fs::write(dir.path().join("pkgsinfo/.DS_Store"), "not a plist").unwrap();
        let (warnings, errors) = rebuild_catalogs(dir.path()).unwrap();
        assert!(warnings.is_empty());
        assert!(errors.is_empty());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn unknown_builder_revision_is_rejected_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([
            ("python_version", "3.13.1".into()),
            ("os_version", "11".into()),
            ("requirements_path", "requirements.txt".into()),
            ("relocatable_python_sha", "unsupported".into()),
            (
                "RECIPE_CACHE_DIR",
                Value::String(dir.path().display().to_string()),
            ),
        ]);
        assert!(generate_python(&mut env)
            .unwrap_err()
            .contains("Unsupported relocatable_python_sha"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!env.contains_key("python_path"));
    }
}
