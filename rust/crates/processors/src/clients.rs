//! Environment preparation for the fixed privileged-helper protocols.
use super::{io, read_dict, remove, string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn mac() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("Package installation is only supported on macOS".into())
    }
}
/// Package creation runs through the helper on macOS and Russet's builder on
/// Linux.
fn can_build() -> Result<()> {
    use autopkg_platform::backend::{select, Backend, Tool};
    if select(Tool::Pkgbuild) == Backend::Unsupported {
        Err("Package creation is only supported on macOS and Linux".into())
    } else {
        Ok(())
    }
}
fn resolve(env: &Dictionary, relative: &str) -> Result<String> {
    let mut roots = Vec::new();
    for key in ["RECIPE_CACHE_DIR", "RECIPE_DIR"] {
        if let Some(path) = env.get(key).and_then(Value::as_string) {
            roots.push(PathBuf::from(path));
        }
    }
    if let Some(parents) = env.get("PARENT_RECIPES").and_then(Value::as_array) {
        for parent in parents {
            if let Some(path) = parent.as_string().and_then(|p| Path::new(p).parent()) {
                roots.push(path.into());
            }
        }
    }
    for root in roots {
        let path = root.join(relative);
        if path.exists() {
            return Ok(path.to_string_lossy().into_owned());
        }
    }
    Err(format!("Can't find {relative}"))
}
fn request(env: &mut Dictionary) -> Result<Dictionary> {
    let mut request = env
        .get("pkg_request")
        .and_then(Value::as_dictionary)
        .ok_or("pkg_request must be a dictionary")?
        .clone();
    if !request.contains_key("pkgdir") {
        request.insert("pkgdir".into(), string(env, "RECIPE_CACHE_DIR")?.into());
    }
    for key in [
        "pkgroot", "pkgname", "pkgtype", "id", "version", "infofile", "scripts",
    ] {
        if !request.contains_key(key) {
            let value = if let Some(value) = env.get(key) {
                value.clone()
            } else if key == "pkgtype" {
                "flat".into()
            } else if key == "infofile" || key == "scripts" {
                "".into()
            } else {
                return Err(format!("Request key {key} missing"));
            };
            request.insert(key.into(), value);
        }
    }
    if !request.contains_key("chown") {
        request.insert("chown".into(), Value::Array(Vec::new()));
    }
    if !request.contains_key("pkgbuild_args") {
        request.insert(
            "pkgbuild_args".into(),
            env.get("pkgbuild_args")
                .filter(|v| truth(Some(v)))
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        );
    }
    for key in ["pkgroot", "pkgdir", "infofile", "scripts"] {
        let path = string(&request, key)?;
        if !path.is_empty() && !path.starts_with('/') {
            request.insert(key.into(), resolve(env, path)?.into());
        }
    }
    env.insert("pkg_request".into(), request.clone().into());
    Ok(request)
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
fn exists(env: &Dictionary, path: &Path, identifier: &str, version: &str) -> Result<bool> {
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
fn summary(env: &mut Dictionary, key: &str, request: &Dictionary, path: &str) {
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
pub(super) fn package(env: &mut Dictionary) -> Result<()> {
    can_build()?;
    env.remove("pkg_creator_summary_result");
    let request = request(env)?;
    let path = Path::new(string(&request, "pkgdir")?)
        .join(format!("{}.pkg", string(&request, "pkgname")?));
    if exists(
        env,
        &path,
        string(&request, "id")?,
        string(&request, "version")?,
    )? {
        output(
            1,
            "Existing package matches version and identifier, not building.",
        );
        env.insert(
            "pkg_path".into(),
            path.to_string_lossy().into_owned().into(),
        );
        env.insert("new_package_request".into(), false.into());
        return Ok(());
    }
    env.insert("new_package_request".into(), true.into());
    let path = autopkg_helpers::packaging_request(&request)?;
    env.insert("pkg_path".into(), path.clone().into());
    summary(env, "pkg_creator_summary_result", &request, &path);
    Ok(())
}
fn package_app(env: &mut Dictionary, app: &Path) -> Result<()> {
    env.remove("app_pkg_creator_summary_result");
    let info = read_dict(&app.join("Contents/Info.plist"))?;
    if !truth(env.get("version")) {
        let key = string(env, "version_key")?;
        let version = info.get(key).ok_or_else(|| {
            format!("The key '{key}' does not exist in the App Bundle's Info.plist!")
        })?;
        env.insert("version".into(), version.clone());
        output(1, format!("Version: {}", plist::python_str(version)));
    }
    if !truth(env.get("bundleid")) {
        env.insert(
            "bundleid".into(),
            info.get("CFBundleIdentifier")
                .ok_or("Missing CFBundleIdentifier")?
                .clone(),
        );
        output(
            1,
            format!("BundleID: {}", plist::python_str(&env["bundleid"])),
        );
    }
    let path = if truth(env.get("pkg_path")) {
        PathBuf::from(string(env, "pkg_path")?)
    } else {
        Path::new(string(env, "RECIPE_CACHE_DIR")?).join(format!(
            "{}-{}.pkg",
            app.file_stem().unwrap_or_default().to_string_lossy(),
            string(env, "version")?
        ))
    };
    if exists(
        env,
        &path,
        string(env, "bundleid")?,
        string(env, "version")?,
    )? {
        output(
            1,
            "Existing package matches version and identifier, not building.",
        );
        env.insert(
            "pkg_path".into(),
            path.to_string_lossy().into_owned().into(),
        );
        env.insert("new_package_request".into(), false.into());
        return Ok(());
    }
    let payload = Path::new(string(env, "RECIPE_CACHE_DIR")?).join("payload");
    if payload.exists() {
        remove(&payload)?;
    }
    let applications = payload.join("Applications");
    io(fs::create_dir_all(&applications))?;
    super::mode(&applications, "775")?;
    super::copy_tree(
        app,
        &applications.join(app.file_name().ok_or("App has no basename")?),
    )?;
    output(
        1,
        format!(
            "Copied {} to {}",
            app.display(),
            applications
                .join(app.file_name().ok_or("App has no basename")?)
                .display()
        ),
    );
    let mut request = Dictionary::new();
    request.insert(
        "pkgroot".into(),
        payload.to_string_lossy().into_owned().into(),
    );
    request.insert(
        "pkgdir".into(),
        path.parent()
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    request.insert(
        "pkgname".into(),
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    request.insert("pkgtype".into(), "flat".into());
    request.insert("id".into(), env["bundleid"].clone());
    request.insert("version".into(), env["version"].clone());
    request.insert("infofile".into(), "".into());
    request.insert("scripts".into(), "".into());
    let mut ownership = Dictionary::new();
    ownership.insert("path".into(), "Applications".into());
    ownership.insert("user".into(), "root".into());
    ownership.insert("group".into(), "admin".into());
    request.insert("chown".into(), Value::Array(vec![ownership.into()]));
    request.insert(
        "pkgbuild_args".into(),
        env.get("pkgbuild_args")
            .filter(|v| truth(Some(v)))
            .cloned()
            .unwrap_or(Value::Array(Vec::new())),
    );
    env.insert("new_package_request".into(), true.into());
    let path = autopkg_helpers::packaging_request(&request)?;
    io(fs::remove_dir_all(payload))?;
    env.insert("pkg_path".into(), path.clone().into());
    summary(env, "app_pkg_creator_summary_result", &request, &path);
    Ok(())
}
pub(super) fn app(env: &mut Dictionary) -> Result<()> {
    can_build()?;
    let path = env
        .get("app_path")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            env.get("pathname")
                .and_then(Value::as_string)
                .filter(|s| !s.is_empty())
                .map(|s| format!("{s}/*.app"))
        })
        .ok_or("No app_path or pathname specified.")?;
    if let Some((image, inner)) = super::dmg::split(&path) {
        let mut mount = super::dmg::Mount::new(image)?;
        let result = (|| {
            let path = mount.resolve(inner)?;
            let matches = super::matches(&path)?;
            super::package::log_glob("app_path", &path, &matches);
            package_app(
                env,
                matches
                    .first()
                    .ok_or("Error processing app_path with glob")?,
            )
        })();
        return mount.detach().and(result);
    }
    let matches = super::matches(&path)?;
    super::package::log_glob("app_path", &path, &matches);
    package_app(
        env,
        matches
            .first()
            .ok_or("Error processing app_path with glob")?,
    )
}
fn install_summary(env: &mut Dictionary, key: &str, text: &str, data_key: &str, data_value: Value) {
    let mut data = Dictionary::new();
    data.insert(data_key.into(), data_value);
    let mut summary = Dictionary::new();
    summary.insert("summary_text".into(), text.into());
    summary.insert("data".into(), data.into());
    env.insert(key.into(), summary.into());
}
pub(super) fn installer_skip(env: &mut Dictionary) -> bool {
    env.remove("installer_summary_result");
    let should_skip = if env.contains_key("new_package_request") {
        !truth(env.get("new_package_request"))
    } else {
        env.contains_key("download_changed") && !truth(env.get("download_changed"))
    };
    if should_skip {
        output(
            1,
            if env.contains_key("new_package_request") {
                "Skipping installation: no new package."
            } else {
                "Skipping installation: no new download."
            },
        );
        env.insert("install_result".into(), "SKIPPED".into());
    }
    should_skip
}
pub(super) fn install(env: &mut Dictionary) -> Result<()> {
    mac()?;
    if installer_skip(env) {
        return Ok(());
    }
    let matches = super::matches(string(env, "pkg_path")?)?;
    super::package::log_glob("pkg_path", string(env, "pkg_path")?, &matches);
    let package = matches
        .first()
        .ok_or("Error processing pkg_path with glob")?;
    let mut request = Dictionary::new();
    request.insert(
        "package".into(),
        package.to_string_lossy().into_owned().into(),
    );
    request.insert(
        "recipe_cache_dir".into(),
        string(env, "RECIPE_CACHE_DIR")?.into(),
    );
    let result =
        autopkg_helpers::installation_request(&request).unwrap_or_else(|e| format!("ERROR: {e}"));
    output(1, format!("Result: {result}"));
    env.insert("install_result".into(), result.clone().into());
    if result == "DONE" {
        install_summary(
            env,
            "installer_summary_result",
            "The following pkgs were successfully installed:",
            "pkg_path",
            package.to_string_lossy().into_owned().into(),
        );
    }
    Ok(())
}
pub(super) fn install_dmg(env: &mut Dictionary) -> Result<()> {
    mac()?;
    env.remove("install_from_dmg_summary_result");
    if env.contains_key("download_changed") && !truth(env.get("download_changed")) {
        output(1, "Skipping installation: no new download.");
        env.insert("install_result".into(), "SKIPPED".into());
        return Ok(());
    }
    let mut mount = super::dmg::Mount::new(string(env, "dmg_path")?)?;
    let mut request = Dictionary::new();
    request.insert(
        "mount_point".into(),
        mount.path().to_string_lossy().into_owned().into(),
    );
    request.insert(
        "items_to_copy".into(),
        env.get("items_to_copy")
            .ok_or("Missing items_to_copy")?
            .clone(),
    );
    let result =
        autopkg_helpers::installation_request(&request).unwrap_or_else(|e| format!("ERROR: {e}"));
    output(1, format!("Result: {result}"));
    env.insert("install_result".into(), result.clone().into());
    if result == "DONE" {
        install_summary(
            env,
            "install_from_dmg_summary_result",
            "Items from the following disk images were successfully installed:",
            "dmg_path",
            env["dmg_path"].clone(),
        );
    }
    mount.detach()
}
#[cfg(test)]
mod tests {
    use super::*;
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
