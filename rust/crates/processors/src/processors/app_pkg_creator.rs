//! `AppPkgCreator`: build a flat package that installs an app into
//! /Applications, through the same helper as `PkgCreator`.
use crate::clients::{can_build, exists, summary};
use crate::{copy_tree, io, matches, mode, read_dict, remove, string, truth, Result};
use autopkg_platform::processor_output as output;
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    mode(&applications, "775")?;
    copy_tree(
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
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
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
    if let Some((image, inner)) = crate::dmg::split(&path) {
        let mut mount = crate::dmg::Mount::new(image)?;
        let result = (|| {
            let path = mount.resolve(inner)?;
            let matches = matches(&path)?;
            crate::package::log_glob("app_path", &path, &matches);
            package_app(
                env,
                matches
                    .first()
                    .ok_or("Error processing app_path with glob")?,
            )
        })();
        return mount.detach().and(result);
    }
    let matches = matches(&path)?;
    crate::package::log_glob("app_path", &path, &matches);
    package_app(
        env,
        matches
            .first()
            .ok_or("Error processing app_path with glob")?,
    )
}
