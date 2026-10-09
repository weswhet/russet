//! Environment preparation for the fixed privileged-helper protocols.
use super::{io, string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::{Dictionary, Value};
use std::{fs, path::Path, process::Command};
pub(crate) fn mac() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("Package installation is only supported on macOS".into())
    }
}
/// Package creation runs through the helper on macOS and Russet's builder on
/// Linux.
pub(crate) fn can_build() -> Result<()> {
    use autopkg_platform::backend::{select, Backend, Tool};
    if select(Tool::Pkgbuild) == Backend::Unsupported {
        Err("Package creation is only supported on macOS and Linux".into())
    } else {
        Ok(())
    }
}
/// Reads an existing package's PackageInfo, with `xar` on macOS or the
/// native reader elsewhere. Logs and returns `None` when it can't.
fn existing_package_info(env: &Dictionary, path: &Path) -> Result<Option<String>> {
    use autopkg_platform::backend::{select, Backend, Tool};
    if select(Tool::Xar) != Backend::Apple {
        #[cfg(unix)]
        return Ok(
            match russet_xar::Archive::open(path).and_then(|mut a| a.read("PackageInfo", 16 << 20))
            {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Err(error) => {
                    autopkg_platform::processor_output(
                        1,
                        format!("extraction of {} with xar failed: {error}", path.display()),
                    );
                    None
                }
            },
        );
    }
    let cache = Path::new(string(env, "RECIPE_CACHE_DIR")?);
    let output = Command::new("/usr/bin/xar")
        .args(["-x", "-C"])
        .arg(cache)
        .arg("-f")
        .arg(path)
        .arg("PackageInfo")
        .output();
    let info = cache.join("PackageInfo");
    let failed = match &output {
        Ok(result) if !result.status.success() => {
            autopkg_platform::processor_output(
                1,
                format!(
                    "extraction of {} with xar failed: {}",
                    path.display(),
                    String::from_utf8_lossy(&result.stderr)
                ),
            );
            true
        }
        Err(error) => {
            autopkg_platform::processor_output(
                1,
                format!(
                    "xar execution failed with error code {}: {error}",
                    error.raw_os_error().unwrap_or(0)
                ),
            );
            true
        }
        _ => false,
    };
    if failed || !info.exists() {
        if !failed {
            autopkg_platform::processor_output(1, "Failed to parse existing package, as no PackageInfo file could be found in the extracted archive.");
        }
        return Ok(None);
    }
    let document = io(fs::read_to_string(&info))?;
    let _ = fs::remove_file(info);
    Ok(Some(document))
}
pub(crate) fn exists(
    env: &Dictionary,
    path: &Path,
    identifier: &str,
    version: &str,
) -> Result<bool> {
    if !path.exists() || truth(env.get("force_pkg_build")) {
        return Ok(false);
    }
    can_build()?;
    output(
        1,
        format!("Package already exists at path {}.", path.display()),
    );
    let Some(document) = existing_package_info(env, path)? else {
        autopkg_platform::processor_output(1, format!("Removing {}", path.display()));
        io(fs::remove_file(path))?;
        return Ok(false);
    };
    let doc = roxmltree::Document::parse(&document).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let local_version = root
        .attribute("version")
        .ok_or("PackageInfo missing version")?;
    let local_id = root
        .attribute("identifier")
        .ok_or("PackageInfo missing identifier")?;
    Ok(local_version == version && local_id == identifier)
}
pub(crate) fn summary(env: &mut Dictionary, key: &str, request: &Dictionary, path: &str) {
    let mut data = Dictionary::new();
    data.insert("identifier".into(), request["id"].clone());
    data.insert("version".into(), request["version"].clone());
    data.insert("pkg_path".into(), path.into());
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        "The following packages were built:".into(),
    );
    summary.insert(
        "report_fields".into(),
        Value::Array(vec![
            "identifier".into(),
            "version".into(),
            "pkg_path".into(),
        ]),
    );
    summary.insert("data".into(), data.into());
    env.insert(key.into(), summary.into());
}
pub(crate) fn install_summary(
    env: &mut Dictionary,
    key: &str,
    text: &str,
    data_key: &str,
    data_value: Value,
) {
    let mut data = Dictionary::new();
    data.insert(data_key.into(), data_value);
    let mut summary = Dictionary::new();
    summary.insert("summary_text".into(), text.into());
    summary.insert("data".into(), data.into());
    env.insert(key.into(), summary.into());
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::processors::{installer::skip as installer_skip, pkg_creator::request};
    use crate::tests::{env, Temp};
    #[test]
    fn defaults_relative_paths_and_skip_precedence() {
        let t = Temp::new();
        fs::create_dir(t.0.join("root")).unwrap();
        let mut e = env(&[
            ("RECIPE_CACHE_DIR", t.0.to_str().unwrap()),
            ("pkgroot", "root"),
            ("pkgname", "Test"),
            ("id", "org.test"),
            ("version", "1"),
        ]);
        e.insert("pkg_request".into(), Dictionary::new().into());
        let request = request(&mut e).unwrap();
        assert_eq!(
            request["pkgroot"].as_string(),
            Some(t.path("root").as_str())
        );
        assert_eq!(request["pkgtype"].as_string(), Some("flat"));
        assert_eq!(request["scripts"].as_string(), Some(""));
        e.insert("download_changed".into(), false.into());
        assert!(installer_skip(&mut e));
        e.insert("new_package_request".into(), true.into());
        assert!(!installer_skip(&mut e));
        e.insert("new_package_request".into(), false.into());
        e.insert("download_changed".into(), true.into());
        assert!(installer_skip(&mut e));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn cached_package_avoids_helper() {
        let t = Temp::new();
        let root = t.0.join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file"), b"fixture").unwrap();
        let pkg = t.path("Test.pkg");
        assert!(Command::new("/usr/bin/pkgbuild")
            .args([
                "--root",
                root.to_str().unwrap(),
                "--identifier",
                "org.test",
                "--version",
                "1",
                &pkg
            ])
            .output()
            .unwrap()
            .status
            .success());
        let mut e = env(&[
            ("RECIPE_CACHE_DIR", t.0.to_str().unwrap()),
            ("pkgroot", "root"),
            ("pkgname", "Test"),
            ("id", "org.test"),
            ("version", "1"),
        ]);
        e.insert("pkg_request".into(), Dictionary::new().into());
        crate::execute("PkgCreator", &mut e).unwrap();
        assert_eq!(e["new_package_request"].as_boolean(), Some(false));
        assert_eq!(e["pkg_path"].as_string(), Some(pkg.as_str()));
        assert!(!e.contains_key("pkg_creator_summary_result"));
    }
}
