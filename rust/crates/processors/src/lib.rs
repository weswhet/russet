//! Built-in portable processors. Unsupported workflows fail explicitly.
mod archive;
mod clients;
mod community_builders;
mod community_legacy;
mod community_modern;
mod registry;
pub use registry::{
    canonical_name, community_contract, community_source, contract, processor_order,
};
mod dmg;
mod download;
mod download_transport;
mod downloader;
mod package;
#[cfg(any(not(target_os = "macos"), test))]
mod predicate;
mod python_glob;
mod python_regex;
mod sparkle;
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
fn warning(env: &mut Dictionary, message: String) -> Result<()> {
    let recipe = Path::new(string(env, "RECIPE_PATH")?)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let name = [".recipe.yaml", ".recipe.plist", ".recipe"]
        .iter()
        .find_map(|extension| recipe.strip_suffix(extension))
        .unwrap_or(&recipe)
        .to_string();
    let mut data = Dictionary::new();
    data.insert("name".into(), name.into());
    data.insert("warning".into(), message.clone().into());
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        "The following recipes have deprecation warnings:".into(),
    );
    summary.insert(
        "report_fields".into(),
        Value::Array(vec!["name".into(), "warning".into()]),
    );
    summary.insert("data".into(), data.into());
    env.insert("deprecation_summary_result".into(), summary.into());
    autopkg_platform::processor_output(1, format!("WARNING: {message}"));
    Ok(())
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
            | "URLTextSearcher"
            | "MunkiImporter"
            | "AdobeReaderURLProvider"
            | "AdobeAcrobatProUpdateInfoProvider"
    ) {
        prepare(name, env)?;
    }
    match name {
        "AdobeReaderURLProvider" | "AdobeAcrobatProUpdateInfoProvider" => {
            community_legacy::execute_typed(name, env)
        }
        "URLGetter" | "DmgMounter" => Err(ExecutionFailure::unexpected(format!(
            "'{name}' object has no attribute 'input_variables'"
        ))),
        "FileMover" => {
            let source = string(env, "source")?;
            let target = string(env, "target")?;
            python_rename(source, target)
                .map_err(|e| ExecutionFailure::unexpected(e.to_string()))?;
            processor_output(
                name,
                env,
                1,
                &format!("File {source} moved to {target}"),
                true,
            );
            Ok(())
        }
        "URLDownloader" | "URLDownloaderPython" => downloader::execute_typed(name, env),
        "URLTextSearcher" => download::execute_typed(env).map(|_| ()),
        "MunkiImporter" => {
            autopkg_munki::importer::execute_classified(env).map_err(|error| match error {
                autopkg_munki::importer::ImportFailure::Processor(message) => message.into(),
                autopkg_munki::importer::ImportFailure::Unexpected(message) => {
                    ExecutionFailure::unexpected(message)
                }
            })
        }
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
        return download::execute(env);
    }
    // A custom output may overwrite its own selector, so capture its name first.
    let custom_output = (name == "FindAndReplace").then(|| {
        env.get("result_output_var_name")
            .and_then(Value::as_string)
            .unwrap_or("output_string")
            .to_owned()
    });
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
            && community_modern::office_is_deprecated(env));
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
    if name == "Installer" && clients::installer_skip(env) {
        return Ok(());
    }
    if let Some(result) = dmg::run_mounted(name, env, preferences, standalone) {
        return result;
    }
    match canonical_name(name) {
        "AutoPkgSourceFinder" | "GenerateRelocatablePython" | "MakeCatalogsProcessor" => community_builders::execute(canonical_name(name), env, preferences),
        "MSOfficeMacURLandUpdateInfoProvider" | "MozillaURLProvider" | "BarebonesURLProvider" => community_modern::execute(canonical_name(name), env, preferences),
        "AdobeAcrobatProUpdateInfoProvider" | "AdobeFlashURLProvider" | "AdobeReaderURLProvider" | "AdobeReaderRepackager" | "PuppetlabsProductsURLProvider" | "SassafrasK2ClientCustomizer" => community_legacy::execute(canonical_name(name), env, preferences),
        "CodeSignatureVerifier" => {
            let matches = match env.get("input_path") {
                Some(Value::String(pattern)) => python_glob::paths_with_recursion(pattern, false)?,
                _ => Vec::new(),
            };
            autopkg_platform::signature::verify_code_signature(env, matches)
        }
        "SignToolVerifier" => autopkg_platform::signature::verify_authenticode(env),
        "GitHubReleasesInfoProvider" => autopkg_platform::github::execute_with_preferences(env, preferences),
        "SparkleUpdateInfoProvider" => sparkle::execute(env),
        "AppDmgVersioner" => dmg::app_version(env),
        "DmgMounter" => Err("'DmgMounter' object has no attribute 'input_variables'".into()),
        "DmgCreator" => dmg::create(env),
        "Unarchiver" => archive::execute(env),
        "PkgCopier" => package::copy(env),
        "PkgCreator" => clients::package(env),
        "AppPkgCreator" => clients::app(env),
        "Installer" => clients::install(env),
        "InstallFromDMG" => clients::install_dmg(env),
        "ChocolateyPackager" => autopkg_platform::chocolatey::execute(env),
        "PkgPayloadUnpacker" => package::unpack_payload(env),
        "PkgExtractor" => package::extract_bundle(env),
        "PkgInfoCreator" | "FlatPkgPacker" | "FlatPkgUnpacker" => package::execute(name, env),
        "URLDownloader" | "URLDownloaderPython" => downloader::execute(name, env),
        "URLGetter" => Err("'URLGetter' object has no attribute 'input_variables'".into()),
        "URLTextSearcher" => download::execute(env).map(|_| ()),
        "MunkiInfoCreator"
        | "MunkiImporter"
        | "MunkiInstallsItemsCreator"
        | "MunkiSetDefaultCatalog"
        | "MunkiPkginfoMerger"
        | "MunkiOptionalReceiptEditor" => autopkg_munki::execute(name, env),
        "StopProcessingIf" => {
            #[cfg(target_os = "macos")]
            let result = autopkg_platform::predicate(string(env, "predicate")?, env)?;
            #[cfg(not(target_os = "macos"))]
            let result = predicate::evaluate(string(env, "predicate")?, env)?;
            output(env, 1, format!("({}) is {}", string(env, "predicate")?, if result { "True" } else { "False" }));
            env.insert("stop_processing_recipe".into(), result.into());
            Ok(())
        }
        "VariableSetter" | "EndOfCheckPhase" => Ok(()),
        "DeprecationWarning" => warning(
            env,
            env.get("warning_message")
                .and_then(Value::as_string)
                .unwrap_or("### This recipe has been deprecated. It may be removed soon. ###")
                .to_string(),
        ),
        "MunkiCatalogBuilder" => warning(env, "MunkiCatalogBuilder was deprecated in AutoPkg version 2.7.5 and may be removed in a future release.".into()),
        "PackageRequired" => {
            let pkg = string(env, "PKG").map_err(|_| "This recipe requires a package or disk image to be pre-downloaded and supplied to autopkg (\"-p\" command-line switch). This is likely due to required login credentials to download the software.".to_string())?;
            if pkg.is_empty() || !Path::new(pkg).exists() {
                return Err(format!(
                    "Path to package or disk image does not exist: {pkg}"
                ));
            }
            Ok(())
        }
        "FileCreator" => {
            let p = string(env, "file_path")?;
            io(write_python_text(p, string(env, "file_content")?))?;
            output(env, 1, format!("Created file at {p}"));
            if env.contains_key("file_mode") {
                mode(Path::new(p), string(env, "file_mode")?)?;
            }
            Ok(())
        }
        "FileMover" => {
            let source = string(env, "source")?;
            let target = string(env, "target")?;
            io(python_rename(source, target))?;
            output(env, 1, format!("File {source} moved to {target}"));
            Ok(())
        },
        "FindAndReplace" => {
            let result =
                string(env, "input_string")?.replace(string(env, "find")?, string(env, "replace")?);
            let key = env
                .get("result_output_var_name")
                .and_then(Value::as_string)
                .unwrap_or("output_string")
                .to_string();
            output(env, 1, format!("Replacing \"{}\" with \"{}\" in \"{}\" and saving result to \"{key}\" variable.", string(env, "find")?, string(env, "replace")?, string(env, "input_string")?));
            env.insert(key, result.into());
            Ok(())
        }
        "Symlinker" => {
            let source = Path::new(string(env, "source_path")?);
            let dest = Path::new(string(env, "destination_path")?);
            if dest.exists() && truth(env.get("overwrite")) {
                io(fs::remove_file(dest))?;
            }
            symlink(source, dest)?;
            output(env, 1, format!("Symlinked {} to {}", source.display(), dest.display()));
            Ok(())
        }
        "Copier" => {
            let pattern = string(env, "source_path")?;
            let paths = matches(pattern)?;
            let source = paths
                .first()
                .ok_or("Error processing source_path with glob")?;
            if paths.len() > 1 {
                output(env, 1, format!("WARNING: Multiple paths match 'source_path' glob '{pattern}':"));
                for path in &paths { output(env, 1, format!("  - {}", path.display())); }
            }
            if pattern.contains(['*', '?', '[', ']', '!']) { output(env, 1, format!("Using path '{}' matched from globbed '{pattern}'.", source.display())); }
            let dest = Path::new(string(env, "destination_path")?);
            if dest.exists() && truth(env.get("overwrite")) {
                remove(dest)?;
            }
            if source.is_dir() {
                copy_tree(source, dest)?;
                output(env, 1, format!("Copied {} to {}", source.display(), dest.display()));
                Ok(())
            } else {
                let target = if dest.is_dir() {
                    dest.join(source.file_name().ok_or("Source has no filename")?)
                } else {
                    dest.to_path_buf()
                };
                // copyfile does not transfer permissions when the destination is a file.
                let permissions = fs::metadata(&target).ok().map(|m| m.permissions());
                if source.canonicalize().ok() == target.canonicalize().ok() && target.exists() {
                    return Err("Source and destination are the same file".into());
                }
                let bytes = io(fs::read(source))?;
                io(fs::write(&target, bytes))?;
                if dest.is_dir() {
                    io(fs::set_permissions(
                        &target,
                        io(fs::metadata(source))?.permissions(),
                    ))?;
                } else if let Some(p) = permissions {
                    io(fs::set_permissions(&target, p))?;
                }
                output(env, 1, format!("Copied {} to {}", source.display(), dest.display()));
                Ok(())
            }
        }
        "FileFinder" => {
            let method = env
                .get("find_method")
                .and_then(Value::as_string)
                .unwrap_or("glob");
            if method != "glob" {
                return Err(format!("Unsupported find_method: {method}"));
            }
            let mut paths = matches(string(env, "pattern")?)?;
            paths.sort();
            let path = paths.last().ok_or("No matching filename found")?;
            env.insert(
                "found_filename".into(),
                path.to_string_lossy().into_owned().into(),
            );
            output(env, 1, format!("Found file match: '{}' from globbed '{}'", path.display(), string(env, "pattern")?));
            env.insert(
                "found_basename".into(),
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            );
            output(env, 1, format!("Basename match: '{}'", string(env, "found_basename")?));
            Ok(())
        }
        "PathDeleter" => {
            if let Some(Value::String(path)) = env.get("path_list") {
                env.insert("path_list".into(), Value::Array(vec![path.clone().into()]));
            }
            let paths = env
                .get("path_list")
                .and_then(Value::as_array)
                .ok_or("path_list must be an array")?;
            for value in paths {
                let path = Path::new(
                    value
                        .as_string()
                        .ok_or("path_list entries must be strings")?,
                );
                let existed = fs::symlink_metadata(path).is_ok();
                if !existed && !path.exists() {
                    if truth(env.get("continue_on_error")) {
                        output(env, 1, format!("Path does not exist, skipping: {}", path.display()));
                        continue;
                    }
                    return Err(format!("Could not remove {} - it does not exist! Set continue_on_error=True to skip missing paths.", path.display()));
                }
                let directory = path.is_dir() && !path.is_symlink();
                let mut result = remove(path);
                if directory {
                    for (attempt, delay) in [(1, 1), (2, 2)] {
                        if result.is_ok() {
                            break;
                        }
                        output(env, 1, format!("Unable to remove {} (attempt {attempt} of 3); retrying in {delay}s", path.display()));
                        std::thread::sleep(std::time::Duration::from_secs(delay));
                        result = remove(path);
                    }
                    if let Err(error) = &result {
                        if truth(env.get("continue_on_error")) {
                            output(env, 1, format!("Ignoring errors on final removal of {}", path.display()));
                            let _ = remove(path);
                            continue;
                        }
                        return Err(format!("Could not remove {} after 3 attempts: {error}", path.display()));
                    }
                }
                match &result {
                    Ok(()) => output(env, 1, format!("Deleted {}", path.display())),
                    Err(error) if truth(env.get("continue_on_error")) => output(env, 1, format!("Ignoring error removing {}: {error}", path.display())),
                    Err(_) => {},
                }
                if !truth(env.get("continue_on_error")) {
                    result?;
                }
            }
            Ok(())
        }
        "PlistEditor" => {
            let mut data = match env.get("input_plist_path").and_then(Value::as_string) {
                Some(p) if !p.is_empty() => read_dict(Path::new(p))?,
                _ => Dictionary::new(),
            };
            let edits = env
                .get("plist_data")
                .and_then(Value::as_dictionary)
                .ok_or("plist_data must be a dictionary")?;
            for (k, v) in edits {
                data.insert(k.clone(), v.clone());
            }
            Value::Dictionary(data)
                .to_file_xml(string(env, "output_plist_path")?)
                .map_err(|e| e.to_string())?;
            output(env, 1, format!("Updated plist at {}", string(env, "output_plist_path")?));
            Ok(())
        }
        "PlistReader" => {
            let normalized = normalized_path(string(env, "info_path")?);
            let p = info_path(&normalized.to_string_lossy())?;
            output(env, 1, format!("Reading: {}", p.display()));
            let data = read_dict(&p)?;
            let mut default = Dictionary::new();
            default.insert("CFBundleShortVersionString".into(), "version".into());
            let keys = env
                .get("plist_keys")
                .and_then(Value::as_dictionary)
                .unwrap_or(&default)
                .clone();
            env.insert(
                "plist_reader_output_variables".into(),
                Dictionary::new().into(),
            );
            for (key, target) in keys {
                let target = target
                    .as_string()
                    .ok_or("plist_keys values must be strings")?;
                let value = data
                    .get(&key)
                    .ok_or_else(|| {
                        format!(
                            "Key '{key}' could not be found in the plist {}!",
                            p.display()
                        )
                    })?
                    .clone();
                env.insert(target.into(), value.clone());
                output(env, 1, format!("Assigning value of '{}' to output variable '{target}'", plist::python_str(&value)));
                env.get_mut("plist_reader_output_variables")
                    .unwrap()
                    .as_dictionary_mut()
                    .unwrap()
                    .insert(target.into(), value);
            }
            Ok(())
        }
        "Versioner" => {
            let path = string(env, "input_plist_path")?;
            let data = if path.to_lowercase().contains(".zip/")
                || path.to_lowercase().contains(".zip\\")
            {
                archive::zip_plist(path, truth(env.get("skip_single_root_dir")))?
                    .ok_or_else(|| format!("File '{path}' was not found."))?
            } else {
                portable_path(path)?;
                read_dict(Path::new(path))?
            };
            let key = env
                .get("plist_version_key")
                .and_then(Value::as_string)
                .unwrap_or("CFBundleShortVersionString");
            env.insert(
                "version".into(),
                data.get(key)
                    .cloned()
                    .unwrap_or_else(|| "UNKNOWN_VERSION".into()),
            );
            output(env, 1, format!("Found version {} in file {}", plist::python_str(&env["version"]), string(env, "input_plist_path")?));
            Ok(())
        }
        "PkgRootCreator" => {
            let root = PathBuf::from(string(env, "pkgroot")?);
            if fs::symlink_metadata(&root).is_ok() {
                remove(&root)?;
            }
            io(fs::create_dir_all(&root))?;
            output(env, 1, format!("Created {}", root.display()));
            let root = io(root.canonicalize())?;
            let dirs = env
                .get("pkgdirs")
                .and_then(Value::as_dictionary)
                .ok_or("pkgdirs must be a dictionary")?;
            let mut sorted: Vec<_> = dirs.iter().collect();
            sorted.sort_by_key(|(k, _)| *k);
            for (dir, permissions) in sorted {
                output(env, 2, format!("Creating {dir}"));
                let mut relative = PathBuf::new();
                for part in Path::new(dir).components() {
                    use std::path::Component;
                    match part {
                        Component::Normal(p) => relative.push(p),
                        Component::CurDir => (),
                        Component::ParentDir => {
                            if !relative.pop() {
                                return Err(format!("{dir} is outside pkgroot"));
                            }
                        }
                        _ => return Err(format!("{dir} in pkgroot is absolute.")),
                    }
                }
                if relative.as_os_str().is_empty() {
                    return Err(format!("{dir} is outside pkgroot"));
                }
                let path = root.join(relative);
                if path.exists() {
                    return Err(format!("{} already exists", path.display()));
                }
                io(fs::create_dir_all(&path))?;
                mode(
                    &path,
                    permissions
                        .as_string()
                        .ok_or("pkgdirs modes must be strings")?,
                )?;
                output(env, 1, format!("Created {}", visible_path(&path)));
            }
            Ok(())
        }
        _ => Err(format!(
            "Processor {name} is not implemented in this Rust build"
        )),
    }
}

fn write_python_text(path: &str, content: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let content = content.replace('\n', "\r\n");
    fs::write(path, content)
}

#[cfg(not(windows))]
fn python_rename(source: &str, target: &str) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn python_rename(source: &str, target: &str) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // Python os.rename uses MoveFileExW without MOVEFILE_REPLACE_EXISTING.
    // std::fs::rename replaces the destination, which changes Python behavior.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &str| -> std::io::Result<Vec<u16>> {
        if path.contains('\0') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "embedded null character",
            ));
        }
        Ok(std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(Some(0))
            .collect())
    };
    let source = wide(source)?;
    let target = wide(target)?;
    // Both pointers refer to terminated UTF-16 buffers retained through this call.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
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
