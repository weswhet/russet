//! `MunkiImporter`: import a package and its pkginfo into a Munki FileRepo,
//! natively. The catalog is read but never rebuilt here.
//!
//! Inputs and outputs: run `russet processor-info MunkiImporter`, or see
//! `MunkiImporter` in `compatibility/reference.json`.
use crate::{
    metadata::{self, Options},
    FileRepo,
};
use plist::{Dictionary, Value};
use std::{
    fs::OpenOptions,
    path::{Component, Path, PathBuf},
};

fn text<'a>(d: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    d.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("Missing or invalid string input: {key}"))
}
fn truth(v: Option<&Value>) -> bool {
    v.is_some_and(crate::truthy)
}
fn component(value: &str) -> Result<(), String> {
    if value.is_empty()
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(format!("Invalid repository filename component: {value:?}"));
    }
    Ok(())
}
fn inside(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err("Repository path must be relative without parent components".into());
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Munki repo not available at {}: {e}", root.display()))?;
    let result = root.join(relative);
    for parent in result.ancestors() {
        if parent.exists() {
            if !parent
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&canonical_root)
            {
                return Err("Repository path resolves outside FileRepo".into());
            }
            break;
        }
    }
    Ok(result)
}
fn collision(path: &Path, index: usize) -> PathBuf {
    if index == 0 {
        return path.to_owned();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let suffix = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    path.with_file_name(format!("{stem}__{index}{suffix}"))
}
impl FileRepo {
    pub fn copy_package(
        &self,
        info: &Dictionary,
        source: &Path,
        subdirectory: &Path,
    ) -> Result<PathBuf, String> {
        let directory = inside(&self.root, &Path::new("pkgs").join(subdirectory))?;
        let filename = source
            .file_name()
            .ok_or("Package path has no filename")?
            .to_string_lossy();
        component(&filename)?;
        let original = directory.join(filename.as_ref());
        if source == original {
            return Ok(original);
        }
        let version = info.get("version").and_then(Value::as_string).unwrap_or("");
        if !version.is_empty() {
            component(version)?;
        }
        let stem = Path::new(filename.as_ref())
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let filename = if !version.is_empty() && !stem.ends_with(version) {
            let ext = Path::new(filename.as_ref())
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();
            format!("{stem}-{version}{ext}")
        } else {
            filename.into_owned()
        };
        let target = directory.join(filename);
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut index = 0;
        loop {
            let path = collision(&target, index);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut output) => {
                    let result = (|| {
                        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
                        std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
                        std::fs::set_permissions(
                            &path,
                            input.metadata().map_err(|e| e.to_string())?.permissions(),
                        )
                        .map_err(|e| e.to_string())?;
                        Ok(())
                    })();
                    if let Err(error) = result {
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    index += 1;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    pub fn copy_pkginfo(
        &self,
        info: &Dictionary,
        subdirectory: &Path,
        extension: &str,
    ) -> Result<PathBuf, String> {
        let name = text(info, "name")?;
        let version = text(info, "version")?.trim();
        component(name)?;
        component(version)?;
        let extension = extension.trim_matches('.');
        if !extension.is_empty() {
            component(extension)?;
        }
        let directory = inside(&self.root, &Path::new("pkgsinfo").join(subdirectory))?;
        let suffix = if extension.is_empty() {
            String::new()
        } else {
            format!(".{extension}")
        };
        let target = directory.join(format!("{name}-{version}{suffix}"));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut index = 0;
        loop {
            let path = if index == 0 {
                target.clone()
            } else {
                directory.join(format!("{name}-{version}__{index}{suffix}"))
            };
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    if let Err(error) = Value::Dictionary(info.clone()).to_writer_xml(file) {
                        let _ = std::fs::remove_file(&path);
                        return Err(error.to_string());
                    }
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    index += 1;
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}
fn options(env: &Dictionary) -> Result<Options, String> {
    let mut args = Vec::new();
    for (key, flag) in [
        ("munkiimport_pkgname", "--pkgname"),
        ("munkiimport_appname", "--appname"),
        ("uninstaller_pkg_path", "--uninstallerpkg"),
    ] {
        if let Some(v) = env
            .get(key)
            .and_then(Value::as_string)
            .filter(|v| !v.is_empty())
        {
            args.extend([flag.into(), v.into()]);
        }
    }
    if let Some(arguments) = env.get("additional_makepkginfo_options") {
        for arg in arguments
            .as_array()
            .ok_or("additional_makepkginfo_options must be an array")?
        {
            args.push(
                arg.as_string()
                    .ok_or("makepkginfo options must be strings")?
                    .to_owned(),
            );
        }
    }
    Options::parse(&args)
}
#[derive(Debug)]
pub enum ImportFailure {
    Processor(String),
    Unexpected(String),
}
impl From<String> for ImportFailure {
    fn from(message: String) -> Self {
        Self::Processor(message)
    }
}
impl From<&str> for ImportFailure {
    fn from(message: &str) -> Self {
        Self::Processor(message.into())
    }
}
impl std::fmt::Display for ImportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Processor(message) | Self::Unexpected(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for ImportFailure {}

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    execute_classified(env).map_err(|error| error.to_string())
}

pub fn execute_classified(env: &mut Dictionary) -> Result<(), ImportFailure> {
    crate::validate_backend(env)?;
    autopkg_platform::processor_output(1, "Using repo lib: AutoPkgLib");
    autopkg_platform::processor_output(
        1,
        format!(
            "        plugin: {}",
            env.get("MUNKI_REPO_PLUGIN")
                .and_then(Value::as_string)
                .unwrap_or("FileRepo")
        ),
    );
    autopkg_platform::processor_output(
        1,
        format!(
            "          repo: {}",
            env.get("MUNKI_REPO")
                .and_then(Value::as_string)
                .unwrap_or("")
        ),
    );
    let opts = options(env)?;
    if opts.output_mode {
        // The reference command succeeds, then plistlib raises InvalidFileException.
        // Preserve that unexpected-error class without invoking the Python tool.
        return Err(ImportFailure::Unexpected(
            "makepkginfo output-mode options do not produce a metadata plist".into(),
        ));
    }
    let package = PathBuf::from(text(env, "pkg_path")?);
    let root = PathBuf::from(text(env, "MUNKI_REPO")?);
    // Resolve symlinks for containment checks, but preserve the caller's path
    // spelling in processor outputs and reports, as the FileRepo reference does.
    root.canonicalize().map_err(|e| e.to_string())?;
    let subdirectory = PathBuf::from(
        env.get("repo_subdirectory")
            .and_then(Value::as_string)
            .unwrap_or(""),
    );
    inside(&root, &Path::new("pkgs").join(&subdirectory))?;
    inside(&root, &Path::new("pkgsinfo").join(&subdirectory))?;
    let repo = FileRepo::new(&root);
    let mut info = metadata::generate(Some(&package), &opts)?;
    let date_pattern = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();
    if let Some(overrides) = env.get("pkginfo") {
        for (key, value) in overrides
            .as_dictionary()
            .ok_or("pkginfo must be a dictionary")?
        {
            let value = if key == "force_install_after_date" {
                value
                    .as_string()
                    .filter(|s| date_pattern.is_match(s))
                    .and_then(|s| plist::Date::from_xml_format(s).ok())
                    .map(Value::Date)
                    .unwrap_or_else(|| value.clone())
            } else {
                value.clone()
            };
            info.insert(key.clone(), value);
        }
    }
    if let Some(additions) = env.get("metadata_additions") {
        let metadata = info
            .get_mut("_metadata")
            .and_then(Value::as_dictionary_mut)
            .ok_or("Generated metadata does not contain _metadata")?;
        metadata.extend(
            additions
                .as_dictionary()
                .ok_or("metadata_additions must be a dictionary")?
                .clone(),
        );
    }
    if let Some(key) = env
        .get("version_comparison_key")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
    {
        if let Some(installs) = info.get_mut("installs").and_then(Value::as_array_mut) {
            for item in installs {
                let item = item
                    .as_dictionary_mut()
                    .ok_or("Installs item must be a dictionary")?;
                if !item.contains_key(key) {
                    return Err(format!(
                        "version_comparison_key '{key}' could not be found in the installs item"
                    )
                    .into());
                }
                item.insert("version_comparison_key".into(), Value::String(key.into()));
            }
        }
    }
    let name = text(&info, "name")?.to_owned();
    let version = text(&info, "version")?.to_owned();
    component(&name)?;
    component(&version)?;
    let catalogs = info
        .get("catalogs")
        .and_then(Value::as_array)
        .ok_or("catalogs must be an array")?
        .iter()
        .map(|v| {
            v.as_string()
                .map(str::to_owned)
                .ok_or("Catalog names must be strings".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let extension = env
        .get("MUNKI_PKGINFO_FILE_EXTENSION")
        .and_then(Value::as_string)
        .unwrap_or("plist")
        .to_owned();
    if !extension.trim_matches('.').is_empty() {
        component(extension.trim_matches('.'))?;
    }
    let matching = if truth(env.get("force_munkiimport")) {
        Vec::new()
    } else {
        repo.index()?.matching(&info)?
    };
    let architecture = info.get("supported_architectures");
    if let Some(matched) = matching
        .iter()
        .find(|item| item.get("supported_architectures") == architecture)
    {
        let matched = if matching
            .iter()
            .any(|item| !item.contains_key("supported_architectures"))
        {
            &matching[0]
        } else {
            matched
        };
        let path = inside(
            &root,
            &Path::new("pkgs").join(text(matched, "installer_item_location")?),
        )?;
        env.remove("munki_importer_summary_result");
        env.insert("pkginfo_repo_path".into(), "".into());
        env.insert(
            "pkg_repo_path".into(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        env.insert("munki_info".into(), Value::Dictionary(Dictionary::new()));
        env.insert("munki_repo_changed".into(), false.into());
        autopkg_platform::processor_output(
            1,
            format!(
                "Item {} already exists in the munki repo as pkgs/{}.",
                package.file_name().unwrap_or_default().to_string_lossy(),
                text(matched, "installer_item_location")?
            ),
        );
        return Ok(());
    }
    // The frozen Python FileRepo passes None to os.path.join when this optional
    // input is omitted. Preserve its unexpected-error category at the copy
    // boundary, after metadata validation and duplicate detection.
    if !env.contains_key("repo_subdirectory")
        || matches!(env.get("repo_subdirectory"), Some(Value::Null))
    {
        return Err(ImportFailure::Unexpected(
            "join() argument must be str, bytes, or os.PathLike object, not 'NoneType'".into(),
        ));
    }
    let mut icon_data = None;
    let icon = if truth(env.get("extract_icon")) {
        let name = info
            .get("icon_name")
            .and_then(Value::as_string)
            .unwrap_or(&name);
        let name = if Path::new(name).extension().is_none() {
            format!("{name}.png")
        } else {
            name.into()
        };
        let path = inside(&root, &Path::new("icons").join(name))?;
        if path.exists() {
            Some(path)
        } else {
            icon_data = crate::icons::extract(&package, &info)?;
            let generated = inside(
                &root,
                &Path::new("icons").join(format!("{}.png", text(&info, "name")?)),
            )?;
            icon_data.as_ref().map(|_| generated)
        }
    } else {
        None
    };
    let imported = repo.copy_package(&info, &package, &subdirectory)?;
    let relative = imported
        .strip_prefix(root.join("pkgs"))
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    info.insert(
        "installer_item_location".into(),
        Value::String(relative.clone()),
    );
    if let Some(uninstaller) = env
        .get("uninstaller_pkg_path")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
    {
        let path = repo.copy_package(&info, Path::new(uninstaller), &subdirectory)?;
        info.insert(
            "uninstaller_item_location".into(),
            Value::String(
                path.strip_prefix(root.join("pkgs"))
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        info.insert("uninstallable".into(), true.into());
    }
    if let (Some(path), Some(data)) = (&icon, icon_data) {
        std::fs::create_dir_all(path.parent().ok_or("Icon path has no parent")?)
            .map_err(|e| e.to_string())?;
        std::fs::write(path, data).map_err(|e| e.to_string())?;
    }
    let pkginfo = repo.copy_pkginfo(&info, &subdirectory, &extension)?;
    let relative_pkginfo = pkginfo
        .strip_prefix(root.join("pkgsinfo"))
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .into_owned();
    let relative_icon = icon
        .as_ref()
        .map(|p| {
            p.strip_prefix(root.join("icons"))
                .map(|p| p.to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .transpose()?
        .unwrap_or_default();
    env.insert(
        "pkginfo_repo_path".into(),
        Value::String(pkginfo.to_string_lossy().into_owned()),
    );
    env.insert(
        "pkg_repo_path".into(),
        Value::String(imported.to_string_lossy().into_owned()),
    );
    env.insert(
        "pkg_path".into(),
        Value::String(imported.to_string_lossy().into_owned()),
    );
    env.insert(
        "icon_repo_path".into(),
        Value::String(
            icon.map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    );
    env.insert("munki_info".into(), Value::Dictionary(info));
    env.insert("munki_repo_changed".into(), true.into());
    env.insert(
        "munki_importer_summary_result".into(),
        Value::Dictionary(Dictionary::from_iter([
            (
                "summary_text",
                Value::String("The following new items were imported into Munki:".into()),
            ),
            (
                "report_fields",
                Value::Array(
                    [
                        "name",
                        "version",
                        "catalogs",
                        "pkginfo_path",
                        "pkg_repo_path",
                        "icon_repo_path",
                    ]
                    .iter()
                    .map(|s| Value::String((*s).into()))
                    .collect(),
                ),
            ),
            (
                "data",
                Value::Dictionary(Dictionary::from_iter([
                    ("name", Value::String(name)),
                    ("version", Value::String(version)),
                    ("catalogs", Value::String(catalogs.join(","))),
                    ("pkginfo_path", Value::String(relative_pkginfo)),
                    ("pkg_repo_path", Value::String(relative)),
                    ("icon_repo_path", Value::String(relative_icon)),
                ])),
            ),
        ])),
    );
    autopkg_platform::processor_output(
        1,
        format!("Copied pkginfo to: {}", text(env, "pkginfo_repo_path")?),
    );
    autopkg_platform::processor_output(
        1,
        format!("           pkg to: {}", text(env, "pkg_repo_path")?),
    );
    if env.get("extract_icon").is_some_and(crate::truthy) {
        autopkg_platform::processor_output(
            1,
            format!("          icon to: {}", text(env, "icon_repo_path")?),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copy_names_collisions_and_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let repo = FileRepo::new(&repo);
        let source = temp.path().join("App.pkg");
        std::fs::write(&source, "payload").unwrap();
        let info = Dictionary::from_iter([
            ("name", Value::String("App".into())),
            ("version", "1.0".into()),
        ]);
        let one = repo
            .copy_package(&info, &source, Path::new("apps"))
            .unwrap();
        let two = repo
            .copy_package(&info, &source, Path::new("apps"))
            .unwrap();
        assert_eq!(one.file_name().unwrap(), "App-1.0.pkg");
        assert_eq!(two.file_name().unwrap(), "App-1.0__1.pkg");
        assert_eq!(std::fs::read(&one).unwrap(), b"payload");
        assert!(repo
            .copy_package(&info, &source, Path::new("../../escape"))
            .is_err());
        let one = repo
            .copy_pkginfo(&info, Path::new("apps"), "plist")
            .unwrap();
        let two = repo
            .copy_pkginfo(&info, Path::new("apps"), "plist")
            .unwrap();
        assert_eq!(one.file_name().unwrap(), "App-1.0.plist");
        assert_eq!(two.file_name().unwrap(), "App-1.0__1.plist");
    }
    #[test]
    fn unknown_option_rejects_before_repository_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([
            (
                "MUNKI_REPO",
                Value::String(temp.path().to_string_lossy().into_owned()),
            ),
            ("pkg_path", "missing.pkg".into()),
            (
                "additional_makepkginfo_options",
                Value::Array(vec!["--future-option".into()]),
            ),
        ]);
        assert!(execute(&mut env).unwrap_err().contains("Unknown"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires native package tools and pinned Munki development reference"]
    fn native_import_and_duplicate_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("payload");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), "test").unwrap();
        let pkg = temp.path().join("Test.pkg");
        metadata::command(
            "/usr/bin/pkgbuild",
            &[
                "--root".as_ref(),
                root.as_os_str(),
                "--identifier".as_ref(),
                "org.autopkg.rust.import".as_ref(),
                "--version".as_ref(),
                "1.0".as_ref(),
                "--install-location".as_ref(),
                "/".as_ref(),
                pkg.as_os_str(),
            ],
        )
        .unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let mut env = Dictionary::from_iter([
            (
                "MUNKI_REPO",
                Value::String(repo.to_string_lossy().into_owned()),
            ),
            (
                "pkg_path",
                Value::String(pkg.to_string_lossy().into_owned()),
            ),
            ("repo_subdirectory", "apps".into()),
        ]);
        execute(&mut env).unwrap();
        assert_eq!(env["munki_repo_changed"].as_boolean(), Some(true));
        assert!(repo.join("pkgs/apps/Test-1.0.pkg").exists());
        assert!(repo.join("pkgsinfo/apps/Test-1.0.plist").exists());
        std::fs::create_dir(repo.join("catalogs")).unwrap();
        Value::Array(vec![env["munki_info"].clone()])
            .to_file_xml(repo.join("catalogs/all"))
            .unwrap();
        env.insert(
            "pkg_path".into(),
            Value::String(pkg.to_string_lossy().into_owned()),
        );
        execute(&mut env).unwrap();
        assert_eq!(env["munki_repo_changed"].as_boolean(), Some(false));
        assert_eq!(env["pkginfo_repo_path"].as_string(), Some(""));
        assert!(!env.contains_key("munki_importer_summary_result"));
        assert_eq!(
            std::fs::read_dir(repo.join("pkgs/apps")).unwrap().count(),
            1
        );
    }
}
