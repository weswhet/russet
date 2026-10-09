//! `PkgCreator`: build a flat package from a package root, through the
//! `russet-server` helper on macOS or Russet's builder on Linux.
//!
//! Inputs and outputs: run `russet processor-info PkgCreator`, or see
//! `PkgCreator` in `compatibility/reference.json`.
use crate::clients::{can_build, exists, summary};
use crate::{string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

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
pub(crate) fn request(env: &mut Dictionary) -> Result<Dictionary> {
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
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
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
