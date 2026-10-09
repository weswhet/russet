//! Built-in portable processors. Unsupported workflows fail explicitly.
mod clients;
mod community_builders;
mod community_legacy;
mod community_modern;
mod registry;
pub use registry::{
    canonical_name, community_contract, community_source, contract, processor_order,
    russet_contract,
};
mod dmg;
mod download_transport;
mod package;
#[cfg(any(not(target_os = "macos"), test))]
mod predicate;
mod processors;
mod python_glob;
mod python_regex;
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, String>;

pub fn supported() -> &'static [&'static str] {
    &[
        "CodeSignatureVerifier",
        "SignToolVerifier",
        "GitHubReleasesInfoProvider",
        "SparkleUpdateInfoProvider",
        "AppDmgVersioner",
        "Unarchiver",
        "DmgCreator",
        "DmgMounter",
        "PkgCopier",
        "PkgCreator",
        "AppPkgCreator",
        "Installer",
        "InstallFromDMG",
        "ChocolateyPackager",
        "PkgPayloadUnpacker",
        "PkgExtractor",
        "PkgInfoCreator",
        "FlatPkgPacker",
        "FlatPkgUnpacker",
        "URLDownloader",
        "URLDownloaderPython",
        "RussetURLDownloader",
        "URLGetter",
        "URLTextSearcher",
        "MunkiInfoCreator",
        "MunkiImporter",
        "MunkiInstallsItemsCreator",
        "MunkiSetDefaultCatalog",
        "MunkiPkginfoMerger",
        "MunkiOptionalReceiptEditor",
        "StopProcessingIf",
        "VariableSetter",
        "EndOfCheckPhase",
        "PackageRequired",
        "DeprecationWarning",
        "FileCreator",
        "FileMover",
        "FindAndReplace",
        "Symlinker",
        "Copier",
        "FileFinder",
        "PathDeleter",
        "PlistEditor",
        "PlistReader",
        "Versioner",
        "PkgRootCreator",
        "MunkiCatalogBuilder",
        "AdobeAcrobatProUpdateInfoProvider",
        "AdobeFlashURLProvider",
        "AdobeReaderURLProvider",
        "AdobeReaderRepackager",
        "AutoPkgSourceFinder",
        "GenerateRelocatablePython",
        "BarebonesURLProvider",
        "MSOfficeMacURLandUpdateInfoProvider",
        "MozillaURLProvider",
        "MakeCatalogsProcessor",
        "PuppetlabsProductsURLProvider",
        "SassafrasK2ClientCustomizer",
        "com.github.autopkg.AutoPkgGitMaster/GenerateRelocatablePython",
    ]
}
/// Apply the frozen reference manifest's defaults and required-input checks.
pub fn prepare(name: &str, env: &mut Dictionary) -> Result<()> {
    let manifest = contract();
    let processor = manifest["processors"]
        .get(name)
        .ok_or_else(|| format!("Unknown built-in processor: {name}"))?;
    if let Some(inputs) = processor["input_variables"].as_object() {
        let order = processor_order();
        let keys = order[name]["input_variables"]
            .as_array()
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(|field| field[0].as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for key in keys {
            let spec = &inputs[key];
            if !env.contains_key(key) {
                if let Some(default) = spec.get("default") {
                    if name == "Unarchiver" && key == "USE_PYTHON_NATIVE_EXTRACTOR" {
                        // Russet extracts with ditto-compatible code on macOS and
                        // Linux, so Python's zipfile behavior is used only on
                        // Windows. AutoPkg itself defaults to Python off macOS.
                        env.insert(key.to_owned(), cfg!(windows).into());
                    } else if name == "ChocolateyPackager" && key == "installer_path" {
                        // Preserve the opaque sentinel diagnostic without exposing it as a value.
                        static INSTALLER_PATH_SENTINEL: u8 = 0;
                        autopkg_platform::processor_output(2, format!(
                            "No value supplied for installer_path, setting default value of: <autopkglib.ChocolateyPackager.VariableSentinel object at {:p}>",
                            &INSTALLER_PATH_SENTINEL
                        ));
                    } else if name == "SignToolVerifier" && key == "signtool_path" {
                        if let Some(path) = autopkg_platform::signature::signtool_default_path() {
                            env.insert(key.to_owned(), path.to_string_lossy().into_owned().into());
                        } else {
                            env.insert(key.to_owned(), Value::Null);
                        }
                    } else {
                        env.insert(key.to_owned(), json_value(default)?);
                    }
                    if let Some(value) = env.get(key) {
                        autopkg_platform::processor_output(
                            2,
                            format!(
                                "No value supplied for {key}, setting default value of: {}",
                                plist::python_str(value)
                            ),
                        );
                    }
                }
            }
            if spec["required"].as_bool() == Some(true) && !env.contains_key(key) {
                return Err(format!("{name} requires {key}"));
            }
        }
    }
    Ok(())
}
fn json_value(value: &serde_json::Value) -> Result<Value> {
    Ok(match value {
        serde_json::Value::String(s) => s.clone().into(),
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into()
            } else if let Some(u) = n.as_u64() {
                u.into()
            } else {
                Value::Real(n.as_f64().ok_or("Invalid manifest number")?)
            }
        }
        serde_json::Value::Array(a) => {
            Value::Array(a.iter().map(json_value).collect::<Result<_>>()?)
        }
        serde_json::Value::Object(d) => Value::Dictionary(
            d.iter()
                .map(|(k, v)| Ok((k.clone(), json_value(v)?)))
                .collect::<Result<_>>()?,
        ),
        serde_json::Value::Null => Value::Null,
    })
}
fn string<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str> {
    env.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("Missing or invalid string input: {key}"))
}
fn truth(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Boolean(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Dictionary(d)) => !d.is_empty(),
        Some(Value::Integer(i)) => i.as_signed().map(|n| n != 0).unwrap_or(true),
        Some(Value::Real(n)) => *n != 0.0,
        _ => true,
    }
}
fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|e| e.to_string())
}
fn mode(path: &Path, text: &str) -> Result<()> {
    let bits = u32::from_str_radix(text, 8).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        io(fs::set_permissions(path, fs::Permissions::from_mode(bits)))?;
    }
    #[cfg(not(unix))]
    {
        let mut p = io(fs::metadata(path))?.permissions();
        p.set_readonly(bits & 0o200 == 0);
        io(fs::set_permissions(path, p))?;
    }
    Ok(())
}
fn symlink(source: &Path, dest: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        io(std::os::unix::fs::symlink(source, dest))
    }
    #[cfg(windows)]
    {
        if source.is_dir() {
            io(std::os::windows::fs::symlink_dir(source, dest))
        } else {
            io(std::os::windows::fs::symlink_file(source, dest))
        }
    }
}
fn remove(path: &Path) -> Result<()> {
    let m = io(fs::symlink_metadata(path))?;
    if m.is_dir() && !m.file_type().is_symlink() {
        io(fs::remove_dir_all(path))
    } else {
        io(fs::remove_file(path))
    }
}
fn portable_path(path: &str) -> Result<()> {
    for component in Path::new(path).ancestors() {
        if component.is_file()
            && component
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ["dmg", "iso", "zip"].contains(&e.to_lowercase().as_str()))
            && component != Path::new(path)
        {
            return Err(format!(
                "Archive or disk image traversal is not implemented: {}",
                component.display()
            ));
        }
    }
    Ok(())
}
/// An input path as it resolves on a case-insensitive macOS volume. Recipes
/// can name `payload` when the file is `Payload`; on other systems, a path
/// that doesn't exist as written resolves to the one entry per component
/// that differs only in case. Anything else is returned unchanged.
fn macos_path(path: &str) -> String {
    if cfg!(target_os = "macos") || Path::new(path).symlink_metadata().is_ok() {
        return path.to_string();
    }
    autopkg_platform::case_fold::resolve(Path::new(path))
        .and_then(|p| p.into_os_string().into_string().ok())
        .unwrap_or_else(|| path.to_string())
}
fn matches(pattern: &str) -> Result<Vec<PathBuf>> {
    portable_path(pattern)?;
    python_glob::paths(pattern)
}
fn copy_metadata(source: &Path, destination: &Path, symlink: bool) -> Result<()> {
    let metadata = if symlink {
        io(fs::symlink_metadata(source))?
    } else {
        io(fs::metadata(source))?
    };
    let accessed = filetime::FileTime::from_last_access_time(&metadata);
    let modified = filetime::FileTime::from_last_modification_time(&metadata);
    if symlink {
        io(filetime::set_symlink_file_times(
            destination,
            accessed,
            modified,
        ))?;
    } else {
        io(filetime::set_file_times(destination, accessed, modified))?;
    }
    #[cfg(unix)]
    {
        let copy = || -> std::io::Result<()> {
            let names = if symlink {
                xattr::list(source)?
            } else {
                xattr::list_deref(source)?
            };
            for name in names {
                let value = if symlink {
                    xattr::get(source, &name)?
                } else {
                    xattr::get_deref(source, &name)?
                };
                if let Some(value) = value {
                    let result = if symlink {
                        xattr::set(destination, &name, &value)
                    } else {
                        xattr::set_deref(destination, &name, &value)
                    };
                    if let Err(error) = result {
                        if !matches!(
                            error.raw_os_error(),
                            Some(libc::EPERM | libc::ENOTSUP | libc::ENODATA | libc::EINVAL)
                        ) {
                            return Err(error);
                        }
                    }
                }
            }
            Ok(())
        };
        if let Err(error) = copy() {
            if !matches!(
                error.raw_os_error(),
                Some(libc::EPERM | libc::ENOTSUP | libc::ENODATA | libc::EINVAL)
            ) {
                return Err(error.to_string());
            }
        }
    }
    if !symlink {
        io(fs::set_permissions(destination, metadata.permissions()))?;
    }
    Ok(())
}
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    io(fs::create_dir(dest))?;
    for entry in io(fs::read_dir(source))? {
        let entry = io(entry)?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        let metadata = io(fs::symlink_metadata(&from))?;
        if metadata.file_type().is_symlink() {
            symlink(&io(fs::read_link(&from))?, &to)?;
            copy_metadata(&from, &to, true)?;
        } else if metadata.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            io(fs::copy(&from, &to))?;
            copy_metadata(&from, &to, false)?;
        }
    }
    copy_metadata(source, dest, false)
}
fn read_dict(path: &Path) -> Result<Dictionary> {
    Value::from_file(path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?
        .into_dictionary()
        .ok_or_else(|| format!("{} is not a dictionary plist", path.display()))
}
fn bundle_info(path: &Path) -> Result<Option<PathBuf>> {
    let p = path.join("Contents").join("Info.plist");
    if path.is_dir() && p.exists() {
        let d = read_dict(&p)?;
        if !d.is_empty() {
            return Ok(Some(p));
        }
    }
    Ok(None)
}
fn info_path(path: &str) -> Result<PathBuf> {
    portable_path(path)?;
    let path = Path::new(path);
    if !path.exists() {
        return Err(format!("Path '{}' doesn't exist!", path.display()));
    }
    if let Some(p) = bundle_info(path)? {
        return Ok(p);
    }
    if path.extension().is_some_and(|e| e == "plist") {
        return Ok(path.into());
    }
    for entry in io(fs::read_dir(path))? {
        let p = io(entry)?.path();
        if p.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with('.'))
        {
            continue;
        }
        if p.is_symlink() && p.extension().is_none() {
            continue;
        }
        if let Some(p) = bundle_info(&p)? {
            return Ok(p);
        }
    }
    Err("No bundle found in dmg".into())
}
fn normalized_path(path: &str) -> PathBuf {
    use std::path::Component;
    let mut result = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    #[cfg(unix)]
    if path.starts_with("//") && !path.starts_with("///") {
        result = PathBuf::from(format!("/{}", result.display()));
    }
    result
}
fn visible_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return rest.into();
        }
    }
    text.into_owned()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Processor,
    Unexpected,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionFailure {
    pub kind: FailureKind,
    pub message: String,
}
impl ExecutionFailure {
    fn unexpected(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Unexpected,
            message: message.into(),
        }
    }
}
impl From<String> for ExecutionFailure {
    fn from(message: String) -> Self {
        Self {
            kind: FailureKind::Processor,
            message,
        }
    }
}
impl From<&str> for ExecutionFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl std::fmt::Display for ExecutionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for ExecutionFailure {}

/// Preserve the reference's distinction between ProcessorError and uncaught
/// runtime exceptions without inspecting diagnostic text.
pub fn execute_standalone(
    name: &str,
    env: &mut Dictionary,
) -> std::result::Result<(), ExecutionFailure> {
    execute_standalone_context(name, env, None)
}
pub fn execute_standalone_with_preferences(
    name: &str,
    env: &mut Dictionary,
    preferences: &Dictionary,
) -> std::result::Result<(), ExecutionFailure> {
    execute_standalone_context(name, env, Some(preferences))
}
fn execute_standalone_context(
    name: &str,
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
) -> std::result::Result<(), ExecutionFailure> {
    let _output_scope = autopkg_platform::processor_output::scope(name, env, true);
    // Direct typed implementations need preparation here. The generic path
    // prepares in execute_builtin; opaque defaults must not be applied twice.
    if matches!(
        name,
        "URLGetter"
            | "DmgMounter"
            | "FileMover"
            | "URLDownloader"
            | "URLDownloaderPython"
            | "RussetURLDownloader"
            | "URLTextSearcher"
            | "MunkiImporter"
            | "AdobeReaderURLProvider"
            | "AdobeAcrobatProUpdateInfoProvider"
    ) {
        prepare(name, env)?;
    }
    match name {
        "AdobeAcrobatProUpdateInfoProvider" => {
            processors::adobe_acrobat_pro_update_info_provider::execute_typed(env)
        }
        "AdobeReaderURLProvider" => processors::adobe_reader_url_provider::execute_typed(env),
        "URLGetter" | "DmgMounter" => Err(ExecutionFailure::unexpected(format!(
            "'{name}' object has no attribute 'input_variables'"
        ))),
        "FileMover" => processors::file_mover::execute_standalone(env, &|env, level, message| {
            processor_output(name, env, level, &message, true)
        }),
        "URLDownloader" => processors::url_downloader::execute_typed(name, env),
        "URLDownloaderPython" => processors::url_downloader_python::execute_typed(env),
        "RussetURLDownloader" => processors::russet_url_downloader::execute_typed(env),
        "URLTextSearcher" => processors::url_text_searcher::execute_typed(env).map(|_| ()),
        "MunkiImporter" => autopkg_munki::processors::munki_importer::execute_classified(env)
            .map_err(|error| match error {
                autopkg_munki::processors::munki_importer::ImportFailure::Processor(message) => {
                    message.into()
                }
                autopkg_munki::processors::munki_importer::ImportFailure::Unexpected(message) => {
                    ExecutionFailure::unexpected(message)
                }
            }),
        _ => execute_outputs_context(name, env, preferences, true)
            .map(|_| ())
            .map_err(Into::into),
    }
}

/// Execute a built-in while retaining the standalone processor API.
pub fn execute(name: &str, env: &mut Dictionary) -> Result<()> {
    execute_with_outputs(name, env).map(|_| ())
}

/// Return the output names declared by this processor invocation. Most built-ins
/// use the frozen manifest; processors with dynamic outputs supply their actual
/// declarations. This deliberately does not include arbitrary environment edits.
pub fn execute_with_outputs(name: &str, env: &mut Dictionary) -> Result<Vec<String>> {
    execute_outputs_context(name, env, None, false)
}
pub fn execute_with_outputs_and_preferences(
    name: &str,
    env: &mut Dictionary,
    preferences: &Dictionary,
) -> Result<Vec<String>> {
    execute_outputs_context(name, env, Some(preferences), false)
}
fn execute_outputs_context(
    name: &str,
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
    standalone: bool,
) -> Result<Vec<String>> {
    let _output_scope = autopkg_platform::processor_output::scope(name, env, standalone);
    if name == "URLTextSearcher" {
        prepare(name, env)?;
        return processors::url_text_searcher::execute(env);
    }
    // A custom output may overwrite its own selector, so capture its name first.
    let custom_output =
        (name == "FindAndReplace").then(|| processors::find_and_replace::output_name(env));
    execute_builtin(name, env, preferences, standalone)?;
    let manifest = contract();
    let mut outputs: Vec<String> = manifest["processors"][name]["output_variables"]
        .as_object()
        .map(|vars| vars.keys().cloned().collect())
        .unwrap_or_default();
    if let Some(name) = custom_output {
        if !outputs.contains(&name) {
            outputs.push(name);
        }
    }
    let emitted_deprecation = matches!(name, "DeprecationWarning" | "MunkiCatalogBuilder")
        || (name == "MSOfficeMacURLandUpdateInfoProvider"
            && processors::ms_office_mac_url_and_update_info_provider::office_is_deprecated(env));
    if emitted_deprecation
        && !outputs
            .iter()
            .any(|key| key == "deprecation_summary_result")
    {
        outputs.push("deprecation_summary_result".into());
    }
    Ok(outputs)
}

fn processor_output(name: &str, env: &Dictionary, level: i64, message: &str, standalone: bool) {
    let verbosity = env
        .get("verbose")
        .and_then(|v| {
            v.as_signed_integer()
                .or_else(|| v.as_string().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);
    if verbosity >= level {
        if standalone {
            autopkg_platform::text_eprintln!("{name}: {message}");
        } else {
            autopkg_platform::text_println!("{name}: {message}");
        }
    }
}

fn execute_builtin(
    name: &str,
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
    standalone: bool,
) -> Result<()> {
    prepare(name, env)?;
    let output = |env: &Dictionary, level: i64, message: String| {
        processor_output(name, env, level, &message, standalone)
    };
    if name == "Installer" && processors::installer::skip(env) {
        return Ok(());
    }
    if let Some(result) = dmg::run_mounted(name, env, preferences, standalone) {
        return result;
    }
    let output: processors::Output = &output;
    match canonical_name(name) {
        "AdobeAcrobatProUpdateInfoProvider" => {
            processors::adobe_acrobat_pro_update_info_provider::execute(env)
        }
        "AdobeFlashURLProvider" => processors::adobe_flash_url_provider::execute(env),
        "AdobeReaderRepackager" => processors::adobe_reader_repackager::execute(env),
        "AdobeReaderURLProvider" => processors::adobe_reader_url_provider::execute(env),
        "AppDmgVersioner" => processors::app_dmg_versioner::execute(env),
        "AppPkgCreator" => processors::app_pkg_creator::execute(env),
        "AutoPkgSourceFinder" => processors::autopkg_source_finder::execute(env),
        "BarebonesURLProvider" => processors::barebones_url_provider::execute(env),
        "ChocolateyPackager" => processors::chocolatey_packager::execute(env),
        "CodeSignatureVerifier" => processors::code_signature_verifier::execute(env),
        "Copier" => processors::copier::execute(env, output),
        "DeprecationWarning" => processors::deprecation_warning::execute(env),
        "DmgCreator" => processors::dmg_creator::execute(env),
        "DmgMounter" => processors::dmg_mounter::execute(),
        "EndOfCheckPhase" => processors::end_of_check_phase::execute(),
        "FileCreator" => processors::file_creator::execute(env, output),
        "FileFinder" => processors::file_finder::execute(env, output),
        "FileMover" => processors::file_mover::execute(env, output),
        "FindAndReplace" => processors::find_and_replace::execute(env, output),
        "FlatPkgPacker" => processors::flat_pkg_packer::execute(env),
        "FlatPkgUnpacker" => processors::flat_pkg_unpacker::execute(env),
        "GenerateRelocatablePython" => processors::generate_relocatable_python::execute(env),
        "GitHubReleasesInfoProvider" => {
            processors::github_releases_info_provider::execute(env, preferences)
        }
        "InstallFromDMG" => processors::install_from_dmg::execute(env),
        "Installer" => processors::installer::execute(env),
        "MakeCatalogsProcessor" => processors::make_catalogs_processor::execute(env, preferences),
        "MozillaURLProvider" => processors::mozilla_url_provider::execute(env),
        "MSOfficeMacURLandUpdateInfoProvider" => {
            processors::ms_office_mac_url_and_update_info_provider::execute(env)
        }
        "MunkiCatalogBuilder" => processors::munki_catalog_builder::execute(env),
        "MunkiImporter"
        | "MunkiInfoCreator"
        | "MunkiInstallsItemsCreator"
        | "MunkiOptionalReceiptEditor"
        | "MunkiPkginfoMerger"
        | "MunkiSetDefaultCatalog" => autopkg_munki::execute(name, env),
        "PackageRequired" => processors::package_required::execute(env),
        "PathDeleter" => processors::path_deleter::execute(env, output),
        "PkgCopier" => processors::pkg_copier::execute(env),
        "PkgCreator" => processors::pkg_creator::execute(env),
        "PkgExtractor" => processors::pkg_extractor::execute(env),
        "PkgInfoCreator" => processors::pkg_info_creator::execute(env),
        "PkgPayloadUnpacker" => processors::pkg_payload_unpacker::execute(env),
        "PkgRootCreator" => processors::pkg_root_creator::execute(env, output),
        "PlistEditor" => processors::plist_editor::execute(env, output),
        "PlistReader" => processors::plist_reader::execute(env, output),
        "PuppetlabsProductsURLProvider" => {
            processors::puppetlabs_products_url_provider::execute(env)
        }
        "RussetURLDownloader" => processors::russet_url_downloader::execute(env),
        "SassafrasK2ClientCustomizer" => processors::sassafras_k2_client_customizer::execute(env),
        "SignToolVerifier" => processors::sign_tool_verifier::execute(env),
        "SparkleUpdateInfoProvider" => processors::sparkle_update_info_provider::execute(env),
        "StopProcessingIf" => processors::stop_processing_if::execute(env, output),
        "Symlinker" => processors::symlinker::execute(env, output),
        "Unarchiver" => processors::unarchiver::execute(env),
        "URLDownloader" => processors::url_downloader::execute(name, env),
        "URLDownloaderPython" => processors::url_downloader_python::execute(env),
        "URLGetter" => processors::url_getter::execute(),
        "URLTextSearcher" => processors::url_text_searcher::execute(env).map(|_| ()),
        "VariableSetter" => processors::variable_setter::execute(),
        "Versioner" => processors::versioner::execute(env, output),
        _ => Err(format!(
            "Processor {name} is not implemented in this Rust build"
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plist_paths_normalize_without_resolving_symlinks() {
        assert_eq!(
            normalized_path("a/../missing.plist"),
            PathBuf::from("missing.plist")
        );
        assert_eq!(
            normalized_path("../../missing.plist"),
            PathBuf::from("../../missing.plist")
        );
        #[cfg(windows)]
        {
            assert_eq!(
                normalized_path("C:/recipes/../missing.plist").to_string_lossy(),
                r"C:\missing.plist"
            );
            assert_eq!(
                visible_path(Path::new(r"\\?\C:\pkgroot\Applications")),
                r"C:\pkgroot\Applications"
            );
            assert_eq!(
                visible_path(Path::new(r"\\?\UNC\server\share\App")),
                r"\\server\share\App"
            );
        }
        #[cfg(unix)]
        assert_eq!(
            normalized_path("//root/../missing.plist").to_string_lossy(),
            "//missing.plist"
        );
    }
    pub(super) struct Temp(pub(super) PathBuf);
    impl Temp {
        pub(super) fn new() -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "autopkg-processors-{}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        pub(super) fn path(&self, name: &str) -> String {
            self.0.join(name).to_string_lossy().into_owned()
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    pub(super) fn env(pairs: &[(&str, &str)]) -> Dictionary {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect()
    }
    #[test]
    fn literal_replacement_and_custom_output() {
        let mut e = env(&[
            ("input_string", "a.*a.*"),
            ("find", ".*"),
            ("replace", "$x"),
            ("result_output_var_name", "result"),
        ]);
        execute("FindAndReplace", &mut e).unwrap();
        assert_eq!(e["result"].as_string(), Some("a$xa$x"));
        assert!(!e.contains_key("output_string"));
    }
    #[test]
    fn create_move_find_delete_unicode() {
        let t = Temp::new();
        let a = t.path("é-a");
        let b = t.path("é-b");
        let mut e = env(&[
            ("file_path", &a),
            ("file_content", "hello λ"),
            ("file_mode", "640"),
        ]);
        execute("FileCreator", &mut e).unwrap();
        e.insert("source".into(), a.into());
        e.insert("target".into(), b.clone().into());
        execute("FileMover", &mut e).unwrap();
        e.insert("pattern".into(), t.path("é-*").into());
        execute("FileFinder", &mut e).unwrap();
        assert_eq!(e["found_basename"].as_string(), Some("é-b"));
        e.insert("path_list".into(), b.clone().into());
        execute("PathDeleter", &mut e).unwrap();
        assert!(e["path_list"].as_array().is_some());
        assert!(!Path::new(&b).exists());
        assert!(execute("PathDeleter", &mut e).is_err());
        e.insert("continue_on_error".into(), true.into());
        execute("PathDeleter", &mut e).unwrap();
    }
    #[test]
    fn plist_types_and_shallow_merge() {
        let t = Temp::new();
        let p = t.path("test.plist");
        let mut original = Dictionary::new();
        original.insert("CFBundleShortVersionString".into(), "2.3".into());
        original.insert("blob".into(), Value::Data(vec![0, 255]));
        Value::Dictionary(original).to_file_binary(&p).unwrap();
        let mut e = env(&[
            ("input_plist_path", &p),
            ("output_plist_path", &p),
            ("info_path", &p),
        ]);
        let mut edits = Dictionary::new();
        edits.insert("flag".into(), true.into());
        e.insert("plist_data".into(), edits.into());
        execute("PlistEditor", &mut e).unwrap();
        assert_eq!(
            read_dict(Path::new(&p)).unwrap()["blob"].as_data(),
            Some(&[0, 255][..])
        );
        execute("PlistReader", &mut e).unwrap();
        assert_eq!(e["version"].as_string(), Some("2.3"));
        assert_eq!(
            e["plist_reader_output_variables"].as_dictionary().unwrap()["version"],
            e["version"]
        );
        e.insert("plist_version_key".into(), "missing".into());
        execute("Versioner", &mut e).unwrap();
        assert_eq!(e["version"].as_string(), Some("UNKNOWN_VERSION"));
    }
    #[test]
    fn root_rejects_escape() {
        let t = Temp::new();
        let mut e = env(&[("pkgroot", &t.path("root"))]);
        let mut dirs = Dictionary::new();
        dirs.insert("../escape".into(), "755".into());
        e.insert("pkgdirs".into(), dirs.into());
        assert!(execute("PkgRootCreator", &mut e)
            .unwrap_err()
            .contains("outside"));
        assert!(!t.0.join("escape").exists());
    }
    #[cfg(unix)]
    #[test]
    fn copy_and_delete_preserve_link_target() {
        let t = Temp::new();
        let src = t.0.join("src");
        fs::create_dir(&src).unwrap();
        let target = t.0.join("target");
        fs::write(&target, "kept").unwrap();
        std::os::unix::fs::symlink(&target, src.join("link")).unwrap();
        let mut e = env(&[
            ("source_path", src.to_str().unwrap()),
            ("destination_path", &t.path("dest")),
        ]);
        execute("Copier", &mut e).unwrap();
        assert!(t.0.join("dest/link").is_symlink());
        e.insert("path_list".into(), t.path("dest/link").into());
        execute("PathDeleter", &mut e).unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "kept");
    }
    #[cfg(unix)]
    #[test]
    fn recursive_copy_preserves_timestamps_and_xattrs() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let file = source.join("file");
        fs::write(&file, b"metadata").unwrap();
        let time = filetime::FileTime::from_unix_time(1_600_000_000, 0);
        filetime::set_file_times(&file, time, time).unwrap();
        let name = if cfg!(target_os = "linux") {
            "user.org.autopkg.fixture"
        } else {
            "org.autopkg.fixture"
        };
        xattr::set(&file, name, b"value").unwrap();
        let destination = temp.0.join("dest");
        copy_tree(&source, &destination).unwrap();
        let copied = destination.join("file");
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&fs::metadata(&copied).unwrap()),
            time
        );
        assert_eq!(xattr::get(copied, name).unwrap(), Some(b"value".to_vec()));
    }
    #[test]
    fn unsupported_never_succeeds() {
        assert!(execute("CustomProcessor", &mut Dictionary::new()).is_err());
        assert!(execute("FileCreator", &mut Dictionary::new()).is_err());
    }
}

#[cfg(test)]
#[test]
fn standalone_opaque_default_is_logged_once() {
    const CHILD: &str = "AUTOPKG_TEST_OPAQUE_DEFAULT_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let mut env = Dictionary::from_iter([
            ("id", Value::String("fixture".into())),
            ("version", "1.0".into()),
            ("title", "Fixture".into()),
            ("authors", "Fixture".into()),
            ("description", "Fixture".into()),
            ("installer_type", "zip".into()),
            ("verbose", 2.into()),
            (
                "chocoexe_path",
                "/nonexistent-autopkg-test/choco.exe".into(),
            ),
        ]);
        assert!(execute_standalone("ChocolateyPackager", &mut env).is_err());
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "standalone_opaque_default_is_logged_once",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .matches("No value supplied for installer_path")
            .count(),
        1
    );
}

#[cfg(test)]
#[test]
fn frozen_defaults_and_required_inputs() {
    let mut env = Dictionary::new();
    assert_eq!(
        prepare("FileCreator", &mut env).unwrap_err(),
        "FileCreator requires file_path"
    );
    env.insert("file_path".into(), "/unused/fixture".into());
    assert_eq!(
        prepare("FileCreator", &mut env).unwrap_err(),
        "FileCreator requires file_content"
    );
    env.insert("input_string".into(), "input".into());
    env.insert("find".into(), "in".into());
    env.insert("replace".into(), "out".into());
    prepare("FindAndReplace", &mut env).unwrap();
    assert_eq!(
        env["result_output_var_name"].as_string(),
        Some("output_string")
    );
    prepare("Unarchiver", &mut env).unwrap();
    assert_eq!(
        env["USE_PYTHON_NATIVE_EXTRACTOR"].as_boolean(),
        Some(cfg!(windows))
    );
}

#[test]
fn null_defaults_are_present_and_false() {
    let mut env = Dictionary::from_iter([("pkg_request", Value::Dictionary(Dictionary::new()))]);
    prepare("PkgCreator", &mut env).unwrap();
    assert!(env["pkgbuild_args"].is_null());
    assert!(!truth(env.get("pkgbuild_args")));
    env.insert("pkgbuild_args".into(), Value::Array(vec!["--quiet".into()]));
    prepare("PkgCreator", &mut env).unwrap();
    assert_eq!(env["pkgbuild_args"].as_array().unwrap().len(), 1);
    let mut env = Dictionary::from_iter([("nullable", Value::Null)]);
    env.insert("predicate".into(), "nullable == NULL".into());
    assert!(predicate::evaluate("nullable == NULL", &env).unwrap());
}

#[test]
fn standalone_failure_kinds_come_from_operations() {
    let mut env = Dictionary::new();
    assert_eq!(
        execute_standalone("FileMover", &mut env).unwrap_err().kind,
        FailureKind::Processor
    );
    let temp = tempfile::tempdir().unwrap();
    env.insert(
        "source".into(),
        temp.path()
            .join("missing")
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    env.insert(
        "target".into(),
        temp.path()
            .join("destination")
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    assert_eq!(
        execute_standalone("FileMover", &mut env).unwrap_err().kind,
        FailureKind::Unexpected
    );
    assert_eq!(
        execute_standalone("URLGetter", &mut Dictionary::new())
            .unwrap_err()
            .kind,
        FailureKind::Unexpected
    );
}

#[cfg(test)]
mod platform_file_compatibility_tests {
    use super::*;

    #[test]
    fn text_file_newline_translation_matches_python_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("café.txt");
        let mut env = Dictionary::new();
        env.insert("file_path".into(), path.to_str().unwrap().into());
        env.insert("file_content".into(), "héllo\nexisting\r\n".into());
        execute("FileCreator", &mut env).unwrap();
        let expected = if cfg!(windows) {
            "héllo\r\nexisting\r\r\n"
        } else {
            "héllo\nexisting\r\n"
        };
        assert_eq!(fs::read(path).unwrap(), expected.as_bytes());
    }

    #[test]
    fn rename_existing_file_matches_python_platform() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        fs::write(&source, "new").unwrap();
        fs::write(&target, "old").unwrap();
        let mut env = Dictionary::new();
        env.insert("source".into(), source.to_str().unwrap().into());
        env.insert("target".into(), target.to_str().unwrap().into());
        let result = execute_standalone("FileMover", &mut env);
        if cfg!(windows) {
            assert_eq!(result.unwrap_err().kind, FailureKind::Unexpected);
            assert_eq!(fs::read(source).unwrap(), b"new");
            assert_eq!(fs::read(target).unwrap(), b"old");
        } else {
            result.unwrap();
            assert!(!source.exists());
            assert_eq!(fs::read(target).unwrap(), b"new");
        }
    }
}
