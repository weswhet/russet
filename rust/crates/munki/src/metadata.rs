//! Native metadata targeting Munki 7.2.0.5787. The pinned option manifest is
//! authoritative; unknown options are rejected before inspecting installer files.
use autopkg_platform::github::compare_versions;
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub values: BTreeMap<String, Vec<String>>,
    pub flags: BTreeSet<String>,
    pub output_mode: bool,
}
impl Options {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key)?.last().map(String::as_str)
    }
    pub fn all(&self, key: &str) -> &[String] {
        self.values.get(key).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
    pub fn parse(arguments: &[String]) -> Result<Self, String> {
        // ArgumentParser's help mode succeeds without producing a metadata plist.
        if arguments.iter().any(|arg| arg == "--help" || arg == "-h") {
            return Ok(Self {
                output_mode: true,
                ..Self::default()
            });
        }
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/munki-makepkginfo-options.json"
        ))
        .map_err(|e| e.to_string())?;
        let mut aliases = BTreeMap::new();
        for option in reference["options"]
            .as_array()
            .ok_or("Invalid pinned Munki options")?
        {
            for flag in option["flags"]
                .as_array()
                .ok_or("Invalid pinned Munki flags")?
            {
                aliases.insert(
                    flag.as_str().unwrap(),
                    (
                        option["property"].as_str().unwrap(),
                        option["kind"].as_str() == Some("Flag"),
                    ),
                );
            }
        }
        let mut result = Self::default();
        let mut args = arguments.iter();
        while let Some(arg) = args.next() {
            let (flag, inline) = arg
                .split_once('=')
                .map(|(a, b)| (a, Some(b)))
                .unwrap_or((arg, None));
            let &(property, is_flag) = aliases
                .get(flag)
                .ok_or_else(|| format!("Unknown makepkginfo option: {flag}"))?;
            if is_flag {
                if inline.is_some() {
                    return Err(format!("Flag {flag} does not take a value"));
                }
                if property == "version" {
                    result.output_mode = true;
                }
                if flag == "--no-print-warnings" {
                    result.flags.insert("noPrintWarnings".into());
                } else {
                    if property == "printWarnings" {
                        result.flags.remove("noPrintWarnings");
                    }
                    result.flags.insert(property.into());
                }
            } else {
                let value = inline
                    .map(str::to_owned)
                    .or_else(|| args.next().cloned())
                    .ok_or_else(|| format!("Option {flag} requires a value"))?;
                result
                    .values
                    .entry(property.into())
                    .or_default()
                    .push(value);
            }
        }
        for key in [
            "pkgvers",
            "minimumMunkiVersion",
            "minimumOSVersion",
            "maximumOSVersion",
        ] {
            if result
                .get(key)
                .is_some_and(|s| !s.chars().next().is_some_and(|c| c.is_ascii_digit()))
            {
                return Err(format!("{key} must start with a digit"));
            }
        }
        if result
            .get("restartAction")
            .is_some_and(|s| !["RequireRestart", "RecommendRestart", "RequireLogout"].contains(&s))
        {
            return Err("Invalid RestartAction".into());
        }
        if result
            .all("supportedArchitectures")
            .iter()
            .any(|s| !["arm64", "x86_64"].contains(&s.as_str()))
        {
            return Err("Unsupported architecture option".into());
        }
        if result
            .get("installerType")
            .is_some_and(|s| !["copy_from_dmg", "stage_os_installer"].contains(&s))
        {
            return Err("Unknown installer_type".into());
        }
        if let Some(date) = result.get("forceInstallAfterDate") {
            plist::Date::from_xml_format(date)
                .map_err(|_| "Cannot parse force_install_after_date".to_owned())?;
        }
        if let Some(mode) = result.get("mode") {
            if !regex::Regex::new(r"[augo]+[=+-][rstwxXugo]+")
                .unwrap()
                .is_match(mode)
            {
                return Err("mode must be a symbolic mode".into());
            }
        }
        let mut keys = BTreeSet::new();
        for value in result.all("installerEnvironment") {
            let (key, _) = value
                .split_once('=')
                .ok_or("installer-environment must be key=value")?;
            if !keys.insert(key) {
                return Err("installer-environment keys must not repeat".into());
            }
        }
        Ok(result)
    }
}
pub(crate) fn command(program: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    autopkg_platform::run(std::ffi::OsStr::new(program), args).map(|o| o.stdout)
}
pub(crate) fn hash(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|data| format!("{:x}", Sha256::digest(data)))
        .map_err(|e| e.to_string())
}
fn text(d: &Dictionary, key: &str) -> Option<String> {
    d.get(key).and_then(Value::as_string).map(str::to_owned)
}
fn expanded(path: &str) -> PathBuf {
    if path == "~" || path.starts_with("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(path.get(2..).unwrap_or(""))
    } else {
        PathBuf::from(path)
    }
}
fn file_contents(path: &str) -> Result<String, String> {
    std::fs::read_to_string(expanded(path)).map_err(|e| format!("Failed to read {path}: {e}"))
}
fn file_or_string(value: &str) -> Result<String, String> {
    if expanded(value).exists() {
        file_contents(value)
    } else {
        Ok(value.into())
    }
}
fn metadata() -> Result<Dictionary, String> {
    let user = if cfg!(unix) {
        String::from_utf8_lossy(&command("/usr/bin/id", &["-un".as_ref()])?)
            .trim()
            .to_owned()
    } else {
        std::env::var("USERNAME").unwrap_or_default()
    };
    let os = if cfg!(target_os = "macos") {
        String::from_utf8_lossy(&command("/usr/bin/sw_vers", &["-productVersion".as_ref()])?)
            .trim()
            .to_owned()
    } else {
        std::env::consts::OS.into()
    };
    Ok(Dictionary::from_iter([
        ("created_by", Value::String(user)),
        (
            "creation_date",
            // Munki and Python plist readers exchange whole-second dates.
            // Fractional XML dates emitted by SystemTime::now() are rejected
            // by Python's plistlib even though Rust's plist parser accepts them.
            Value::Date(
                (std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_err(|e| e.to_string())?
                            .as_secs(),
                    ))
                .into(),
            ),
        ),
        (
            "munki_version",
            Value::String(crate::REFERENCE_VERSION.into()),
        ),
        ("os_version", Value::String(os)),
    ]))
}
pub fn generate(installer: Option<&Path>, options: &Options) -> Result<Dictionary, String> {
    if options.output_mode {
        return Err("makepkginfo output-mode options do not produce a metadata plist".into());
    }
    let resolved = installer.map(resolve_installer_input).transpose()?;
    let installer = resolved.as_deref();
    let mut info = if let Some(path) = installer.filter(|p| !p.as_os_str().is_empty()) {
        if !path.exists() {
            return Err(format!("File {} does not exist", path.display()));
        }
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("pkg" | "mpkg") => package(path, options)?,
            Some("dmg" | "iso") => disk_image(path, options)?,
            _ => {
                return Err(format!(
                    "{} is not a supported installer item",
                    path.display()
                ))
            }
        }
    } else {
        let mut info = Dictionary::new();
        if options.flag("nopkg") {
            info.insert("installer_type".into(), "nopkg".into());
        }
        info
    };
    if let Some(path) = installer.filter(|p| !p.as_os_str().is_empty()) {
        info.insert(
            "installer_item_location".into(),
            Value::String(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        if !info.contains_key("uninstall_method")
            && info
                .get("receipts")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        {
            info.insert("uninstallable".into(), true.into());
            info.insert("uninstall_method".into(), "removepackages".into());
        }
        if let Some(path) = options.get("uninstalleritem") {
            let path = Path::new(path);
            info.insert("uninstallable".into(), true.into());
            info.insert("uninstall_method".into(), "uninstall_package".into());
            info.insert(
                "uninstaller_item_location".into(),
                Value::String(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ),
            );
            info.insert("uninstaller_item_hash".into(), Value::String(hash(path)?));
            info.insert(
                "uninstaller_item_size".into(),
                Value::Integer(
                    (std::fs::metadata(path).map_err(|e| e.to_string())?.len() / 1024).into(),
                ),
            );
        }
    }
    let catalogs = if options.all("catalog").is_empty() {
        vec![
            autopkg_platform::preference("com.googlecode.munki.munkiimport", "default_catalog")?
                .and_then(Value::into_string)
                .unwrap_or_else(|| "testing".into()),
        ]
    } else {
        options.all("catalog").to_vec()
    };
    info.insert(
        "catalogs".into(),
        Value::Array(catalogs.into_iter().map(Value::String).collect()),
    );
    for (option, key) in [
        ("name", "name"),
        ("displayname", "display_name"),
        ("pkgvers", "version"),
        ("category", "category"),
        ("developer", "developer"),
        ("iconName", "icon_name"),
    ] {
        if let Some(v) = options.get(option) {
            info.insert(key.into(), Value::String(v.into()));
        }
    }
    if let Some(v) = options.get("description") {
        info.insert("description".into(), Value::String(file_or_string(v)?));
    }
    let mut installs = Vec::new();
    for file in options.all("file") {
        let file = if file != "/" {
            file.strip_suffix('/').unwrap_or(file)
        } else {
            file
        };
        if Path::new(file).exists() {
            installs.push(Value::Dictionary(
                crate::processors::munki_installs_items_creator::create_item(Path::new(file))?,
            ));
        }
    }
    if !installs.is_empty() {
        info.insert("installs".into(), Value::Array(installs));
    }
    let mut minos = text(&info, "minimum_os_version");
    for item in info
        .get("installs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_dictionary)
    {
        if info.get("installer_type").and_then(Value::as_string) == Some("stage_os_installer") {
            break;
        }
        if let Some(v) = text(item, "minosversion") {
            if minos
                .as_deref()
                .is_none_or(|m| compare_versions(&v, m).is_gt())
            {
                minos = Some(v);
            }
        }
    }
    if let Some(v) = minos {
        info.insert("minimum_os_version".into(), Value::String(v));
    }
    for (option, key) in [
        ("installcheckScript", "installcheck_script"),
        ("uninstallcheckScript", "uninstallcheck_script"),
        ("preinstallScript", "preinstall_script"),
        ("postinstallScript", "postinstall_script"),
        ("preuninstallScript", "preuninstall_script"),
        ("postuninstallScript", "postuninstall_script"),
        ("uninstallScript", "uninstall_script"),
        ("versionScript", "version_script"),
    ] {
        if let Some(path) = options.get(option) {
            info.insert(key.into(), Value::String(file_contents(path)?));
        }
    }
    if options.get("uninstallScript").is_some() {
        info.insert("uninstallable".into(), true.into());
        info.insert("uninstall_method".into(), "uninstall_script".into());
    }
    if installer.is_some_and(|p| !p.as_os_str().is_empty()) || options.flag("nopkg") {
        info.insert("_metadata".into(), Value::Dictionary(metadata()?));
        info.insert(
            "autoremove".into(),
            Value::Boolean(options.flag("autoremove")),
        );
    }
    for (option, key) in [
        ("minimumMunkiVersion", "minimum_munki_version"),
        ("minimumOSVersion", "minimum_os_version"),
        ("maximumOSVersion", "maximum_os_version"),
        ("forceInstallAfterDate", "force_install_after_date"),
        ("restartAction", "RestartAction"),
    ] {
        if let Some(v) = options.get(option) {
            info.insert(key.into(), Value::String(v.into()));
        }
    }
    for (flag, key) in [
        ("onDemand", "OnDemand"),
        ("unattendedInstall", "unattended_install"),
        ("unattendedUninstall", "unattended_uninstall"),
    ] {
        if options.flag(flag) {
            info.insert(key.into(), true.into());
        }
    }
    for (option, key) in [
        ("supportedArchitectures", "supported_architectures"),
        ("updateFor", "update_for"),
        ("requires", "requires"),
        ("blockingApplication", "blocking_applications"),
    ] {
        if !options.all(option).is_empty() {
            info.insert(
                key.into(),
                Value::Array(
                    options
                        .all(option)
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
    }
    if let Some(v) = options.get("uninstallMethod") {
        info.insert("uninstall_method".into(), Value::String(v.into()));
        info.insert("uninstallable".into(), true.into());
    }
    if !options.all("installerEnvironment").is_empty() {
        info.insert(
            "installer_environment".into(),
            Value::Dictionary(
                options
                    .all("installerEnvironment")
                    .iter()
                    .map(|line| {
                        let (k, v) = line.split_once('=').unwrap();
                        (k.to_owned(), Value::String(v.into()))
                    })
                    .collect(),
            ),
        );
    }
    if let Some(v) = options.get("notes") {
        info.insert("notes".into(), Value::String(file_or_string(v)?));
    }
    Ok(info)
}

/// A mounted image root is accepted by makepkginfo as an alias for its image.
/// Nested directories and ordinary package bundles retain their original path.
pub fn resolve_installer_input(path: &Path) -> Result<PathBuf, String> {
    use autopkg_platform::backend::{select, Backend, Tool};
    if select(Tool::Hdiutil) != Backend::Apple
        || !path.is_dir()
        || path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("pkg") || s.eq_ignore_ascii_case("mpkg"))
    {
        return Ok(path.to_owned());
    }
    let output = command("/usr/bin/hdiutil", &["info".as_ref(), "-plist".as_ref()])?;
    let info = autopkg_platform::dmg::parse_hdiutil_plist(&output)?;
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    for image in info
        .as_dictionary()
        .and_then(|d| d.get("images"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_dictionary)
    {
        let Some(image_path) = image.get("image-path").and_then(Value::as_string) else {
            continue;
        };
        for entity in image
            .get("system-entities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_dictionary)
        {
            if entity
                .get("mount-point")
                .and_then(Value::as_string)
                .and_then(|p| Path::new(p).canonicalize().ok())
                .as_ref()
                == Some(&canonical)
            {
                return Ok(PathBuf::from(image_path));
            }
        }
    }
    Ok(path.to_owned())
}

pub fn disk_image(path: &Path, options: &Options) -> Result<Dictionary, String> {
    let mut mount = crate::mount::Mount::new(&path.to_string_lossy())?;
    let mut info = Dictionary::new();
    if let Some(pkg) = options.get("pkgname") {
        info = package(Path::new(&mount.resolve(pkg)?), options)?;
        info.insert("package_path".into(), Value::String(pkg.into()));
    } else if options.get("item").is_none() {
        for entry in std::fs::read_dir(mount.path()).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| ["pkg", "mpkg"].contains(&s.to_lowercase().as_str()))
            {
                info = package(&path, options)?;
                break;
            }
        }
    }
    if info.is_empty() {
        let item = if let Some(item) = options.get("item") {
            item.to_owned()
        } else {
            let mut app = None;
            for entry in std::fs::read_dir(mount.path()).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.is_dir()
                    && (path.extension().and_then(|s| s.to_str()) == Some("app")
                        || crate::processors::munki_installs_items_creator::create_item(&path)?
                            .get("type")
                            .and_then(Value::as_string)
                            == Some("application"))
                {
                    app = path.file_name().map(|s| s.to_string_lossy().into_owned());
                    break;
                }
            }
            app.ok_or("No application found on disk image")?
        };
        let app = PathBuf::from(mount.resolve(&item)?);
        if !app.exists() {
            return Err(format!("Disk image item {item} does not exist"));
        }
        let mut install = crate::processors::munki_installs_items_creator::create_item(&app)?;
        let destination = options.get("destinationpath").unwrap_or("/Applications");
        let destination_item = options.get("destitemname").unwrap_or(&item);
        let basename = Path::new(destination_item).file_name().unwrap_or_default();
        install.insert(
            "path".into(),
            Value::String(
                Path::new(destination)
                    .join(basename)
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        let name = text(&install, "CFBundleName").unwrap_or_else(|| {
            Path::new(&item)
                .with_extension("")
                .to_string_lossy()
                .into_owned()
        });
        let key = install
            .get("version_comparison_key")
            .and_then(Value::as_string)
            .unwrap_or("CFBundleShortVersionString");
        let version = text(&install, key).unwrap_or_else(|| "0.0.0.0.0".into());
        info.insert("name".into(), Value::String(name));
        info.insert("version".into(), Value::String(version));
        if let Some(v) = text(&install, "minosversion") {
            info.insert("minimum_os_version".into(), Value::String(v));
        }
        info.insert(
            "installs".into(),
            Value::Array(vec![Value::Dictionary(install)]),
        );
        info.insert("installer_type".into(), "copy_from_dmg".into());
        let mut copy = Dictionary::from_iter([
            ("source_item", Value::String(item)),
            ("destination_path", Value::String(destination.into())),
        ]);
        for (option, key) in [
            ("destitemname", "destination_item"),
            ("user", "user"),
            ("group", "group"),
            ("mode", "mode"),
        ] {
            if let Some(value) = options.get(option) {
                copy.insert(key.into(), Value::String(value.into()));
            }
        }
        info.insert(
            "items_to_copy".into(),
            Value::Array(vec![Value::Dictionary(copy)]),
        );
        info.insert("uninstallable".into(), true.into());
        info.insert("uninstall_method".into(), "remove_copied_items".into());
        if options.get("installerType") != Some("copy_from_dmg")
            && app.join("Contents/Resources/startosinstall").exists()
        {
            info.extend(crate::osinstaller::stage_metadata(&app)?);
        }
    }
    info.insert("installer_item_hash".into(), Value::String(hash(path)?));
    info.insert(
        "installer_item_size".into(),
        Value::Integer((std::fs::metadata(path).map_err(|e| e.to_string())?.len() / 1024).into()),
    );
    if !options.flag("noPrintWarnings")
        && crate::tools::image_format(path)?
            .is_some_and(|s| ["UDSB", "UDSP", "UDRW", "RdWr"].contains(&s.as_str()))
    {
        info.insert("installer_item_hash".into(), "N/A".into());
    }
    mount.detach()?;
    Ok(info)
}

fn receipt(node: roxmltree::Node<'_, '_>) -> Option<Dictionary> {
    let id = node.attribute("identifier")?;
    let version = node.attribute("version")?;
    let mut receipt = Dictionary::from_iter([
        ("packageid", Value::String(id.into())),
        ("version", Value::String(version.into())),
    ]);
    if let Some(min) = node.attribute("minimumSystemVersion") {
        receipt.insert("minimum_os_version".into(), Value::String(min.into()));
    }
    if let Some(size) = node
        .children()
        .find(|n| n.has_tag_name("payload"))
        .and_then(|n| n.attribute("installKBytes"))
        .and_then(|s| s.parse::<u64>().ok())
    {
        receipt.insert("installed_size".into(), Value::Integer(size.into()));
    }
    Some(receipt)
}
pub fn package(path: &Path, options: &Options) -> Result<Dictionary, String> {
    if path.is_dir() {
        return crate::bundle::package(path, options);
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let mut receipts = Vec::new();
    let mut version = String::new();
    let mut minimum = String::new();
    let mut distribution = None;
    for (entry, xml) in crate::tools::metadata_documents(&path)? {
        if entry.ends_with("Distribution") {
            if distribution.is_none() {
                distribution = Some(xml);
            }
            continue;
        }
        let document = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
        if let Some(receipt) = document
            .descendants()
            .find(|n| n.has_tag_name("pkg-info"))
            .and_then(receipt)
        {
            if let Some(v) = text(&receipt, "version") {
                if compare_versions(&v, &version).is_gt() {
                    version = v;
                }
            }
            if receipt.contains_key("installed_size") {
                receipts.push(Value::Dictionary(receipt));
            }
        }
    }
    if let Some(xml) = distribution {
        let doc = roxmltree::Document::parse(&xml).map_err(|e| e.to_string())?;
        version = doc
            .descendants()
            .find(|n| n.has_tag_name("product"))
            .and_then(|n| n.attribute("version"))
            .unwrap_or("")
            .into();
        if let Some(volume) = doc.descendants().find(|n| n.has_tag_name("volume-check")) {
            if let Some(allowed) = volume
                .children()
                .find(|n| n.has_tag_name("allowed-os-versions"))
            {
                minimum = allowed
                    .children()
                    .filter(|n| n.has_tag_name("os-version"))
                    .filter_map(|n| n.attribute("min"))
                    .max_by(|a, b| compare_versions(a, b))
                    .unwrap_or("")
                    .into();
            }
        }
        if receipts.is_empty() {
            let mut references: BTreeMap<String, Dictionary> = BTreeMap::new();
            let mut files = BTreeSet::new();
            for node in doc.descendants().filter(|n| n.has_tag_name("pkg-ref")) {
                let Some(id) = node.attribute("id") else {
                    continue;
                };
                let item = references.entry(id.into()).or_insert_with(|| {
                    Dictionary::from_iter([("packageid", Value::String(id.into()))])
                });
                if let Some(v) = node.attribute("version") {
                    item.insert("version".into(), Value::String(v.into()));
                }
                if let Some(size) = node
                    .attribute("installKBytes")
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    item.insert("installed_size".into(), Value::Integer(size.into()));
                }
                if node.text().is_some_and(|s| !s.is_empty()) {
                    files.insert(id.to_owned());
                }
            }
            receipts = references
                .into_iter()
                .filter(|(id, d)| files.contains(id) && d.contains_key("version"))
                .map(|(_, d)| Value::Dictionary(d))
                .collect();
        }
    }
    if version.is_empty() {
        version = receipts
            .iter()
            .filter_map(Value::as_dictionary)
            .filter_map(|r| r.get("version").and_then(Value::as_string))
            .max_by(|a, b| compare_versions(a, b))
            .unwrap_or("0.0.0.0.0")
            .into();
    }
    if minimum.is_empty() {
        minimum = receipts
            .iter()
            .filter_map(Value::as_dictionary)
            .filter_map(|r| r.get("minimum_os_version").and_then(Value::as_string))
            .max_by(|a, b| compare_versions(a, b))
            .unwrap_or("")
            .into();
    }
    let basename = path.file_stem().unwrap_or_default().to_string_lossy();
    let mut name = basename.to_string();
    for delim in ["--", "-"] {
        if let Some((n, v)) = basename.rsplit_once(delim) {
            if v.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                name = n.into();
                break;
            }
        }
    }
    let size: u64 = receipts
        .iter()
        .filter_map(Value::as_dictionary)
        .filter_map(|r| r.get("installed_size").and_then(Value::as_unsigned_integer))
        .sum();
    let mut info = Dictionary::from_iter([
        ("name", Value::String(name)),
        ("version", Value::String(version)),
        ("installer_item_hash", Value::String(hash(&path)?)),
        (
            "installer_item_size",
            Value::Integer(
                (std::fs::metadata(&path).map_err(|e| e.to_string())?.len() / 1024).into(),
            ),
        ),
    ]);
    if !receipts.is_empty() {
        info.insert("receipts".into(), Value::Array(receipts));
    }
    if size > 0 {
        info.insert("installed_size".into(), Value::Integer(size.into()));
    }
    if !minimum.is_empty() {
        info.insert("minimum_os_version".into(), Value::String(minimum));
    }
    if let Some(action) = crate::tools::restart_action(&path, None)? {
        info.insert("RestartAction".into(), Value::String(action));
    }
    if options.flag("installerChoices") {
        if let Some(choices) = crate::tools::installer_choices(&path)? {
            info.insert("installer_choices_xml".into(), Value::Array(choices));
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::process::Command;
    #[test]
    fn pinned_option_validation_rejects_unknown_and_invalid_values() {
        let parse =
            |args: &[&str]| Options::parse(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>());
        assert!(parse(&["--future-option"]).unwrap_err().contains("Unknown"));
        assert!(parse(&["--installer-type", "stage_os_installer"]).is_ok());
        assert!(parse(&["--installer-type", "unknown"]).is_err());
        assert!(parse(&["--pkgvers", "beta"]).is_err());
        assert!(parse(&["--arch", "i386"]).is_err());
        assert!(parse(&["-E", "A=1", "-E", "A=2"]).is_err());
        for flag in ["--version", "-V", "--help", "-h"] {
            let options = parse(&[flag]).unwrap();
            assert!(options.output_mode);
            assert!(generate(None, &options)
                .unwrap_err()
                .contains("metadata plist"));
        }
        let options = parse(&[
            "--catalog=testing",
            "-c",
            "production",
            "--unattended_install",
            "--name",
            "Test",
        ])
        .unwrap();
        assert_eq!(options.all("catalog"), &["testing", "production"]);
        assert!(options.flag("unattendedInstall"));
    }
    #[test]
    fn metadata_only_scripts_and_options() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("script");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        let args = vec![
            "--name".into(),
            "Test".into(),
            "--pkgvers".into(),
            "2.0".into(),
            "--installcheck-script".into(),
            script.to_string_lossy().into_owned(),
            "--force-install-after-date".into(),
            "2026-10-06T12:00:00Z".into(),
            "--installer-environment".into(),
            "USER=CURRENT_CONSOLE_USER".into(),
        ];
        let info = generate(None, &Options::parse(&args).unwrap()).unwrap();
        assert_eq!(
            info["installcheck_script"].as_string(),
            Some("#!/bin/sh\nexit 0\n")
        );
        assert_eq!(
            info["force_install_after_date"].as_string(),
            Some("2026-10-06T12:00:00Z")
        );
        assert!(!info.contains_key("_metadata"));
    }
    #[cfg(target_os = "macos")]
    fn normalize(mut info: Dictionary) -> Dictionary {
        if let Some(metadata) = info.get_mut("_metadata").and_then(Value::as_dictionary_mut) {
            metadata.remove("creation_date");
        }
        info
    }
    #[cfg(target_os = "macos")]
    fn reference(installer: Option<&Path>, args: &[String]) -> Dictionary {
        let mut command = Command::new("/usr/local/munki/makepkginfo");
        if let Some(path) = installer {
            command.arg(path);
        }
        command.args(args);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Value::from_reader(std::io::Cursor::new(output.stdout))
            .unwrap()
            .into_dictionary()
            .unwrap()
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires pinned Munki 7.2.0.5787 development reference and native packaging tools"]
    fn differential_metadata_options_and_flat_package() {
        let version = Command::new("/usr/local/munki/makepkginfo")
            .arg("--version")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&version.stdout).trim(),
            crate::REFERENCE_VERSION
        );
        let args: Vec<String> = [
            "--nopkg",
            "--name",
            "Test",
            "--pkgvers",
            "2.0",
            "--displayname",
            "Test App",
            "--description",
            "Sample description",
            "--category",
            "Tools",
            "--developer",
            "Example",
            "--icon",
            "test.png",
            "--notes",
            "Example notes",
            "--autoremove",
            "--OnDemand",
            "--unattended-install",
            "--unattended-uninstall",
            "--minimum-munki-version",
            "6.0",
            "--minimum-os-version",
            "12.0",
            "--maximum-os-version",
            "15.0",
            "--arch",
            "arm64",
            "--update-for",
            "OldApp",
            "--requires",
            "Required",
            "--blocking-application",
            "App",
            "--RestartAction",
            "RequireLogout",
            "--uninstall-method",
            "remove_app",
            "--force-install-after-date",
            "2026-10-06T12:00:00Z",
            "-E",
            "USER=CURRENT_CONSOLE_USER",
        ]
        .iter()
        .map(|s| (*s).into())
        .collect();
        assert_eq!(
            normalize(generate(None, &Options::parse(&args).unwrap()).unwrap()),
            normalize(reference(None, &args))
        );
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), "test content").unwrap();
        let pkg = temp.path().join("Test-2.3.pkg");
        command(
            "/usr/bin/pkgbuild",
            &[
                "--root".as_ref(),
                root.as_os_str(),
                "--identifier".as_ref(),
                "org.autopkg.rust.fixture".as_ref(),
                "--version".as_ref(),
                "2.3".as_ref(),
                "--install-location".as_ref(),
                "/".as_ref(),
                pkg.as_os_str(),
            ],
        )
        .unwrap();
        assert_eq!(
            normalize(generate(Some(&pkg), &Options::default()).unwrap()),
            normalize(reference(Some(&pkg), &[]))
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires pinned Munki development reference and hdiutil"]
    fn differential_drag_and_drop_disk_image() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let app = source.join("Test.app/Contents");
        std::fs::create_dir_all(&app).unwrap();
        Value::Dictionary(Dictionary::from_iter([
            ("CFBundleName", Value::String("Test".into())),
            ("CFBundleIdentifier", "org.test".into()),
            ("CFBundleShortVersionString", "2.3".into()),
            ("CFBundleVersion", "42".into()),
            ("LSMinimumSystemVersion", "12.0".into()),
        ]))
        .to_file_xml(app.join("Info.plist"))
        .unwrap();
        let dmg = temp.path().join("Test.dmg");
        command(
            "/usr/bin/hdiutil",
            &[
                "create".as_ref(),
                "-srcfolder".as_ref(),
                source.as_os_str(),
                "-format".as_ref(),
                "UDZO".as_ref(),
                dmg.as_os_str(),
            ],
        )
        .unwrap();
        let args: Vec<String> = [
            "--appname",
            "Test.app",
            "--destinationpath",
            "/Applications/Tools",
            "--destinationitem",
            "Renamed.app",
            "--owner",
            "root",
            "--group",
            "wheel",
            "--mode",
            "a+rx",
        ]
        .iter()
        .map(|s| (*s).into())
        .collect();
        assert_eq!(
            normalize(generate(Some(&dmg), &Options::parse(&args).unwrap()).unwrap()),
            normalize(reference(Some(&dmg), &args))
        );
        let mut mounted = autopkg_platform::dmg::Mount::new(dmg.to_str().unwrap()).unwrap();
        let mounted_path = mounted.path().to_owned();
        let actual = normalize(generate(Some(&mounted_path), &Options::default()).unwrap());
        assert!(
            mounted_path.is_dir(),
            "Rust metadata inspection detached a borrowed mount"
        );
        assert_eq!(actual, normalize(reference(Some(&mounted_path), &[])));
        assert!(
            mounted_path.is_dir(),
            "Reference metadata inspection detached a borrowed mount"
        );
        mounted.detach().unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires pinned Munki reference and native image tools; creates only synthetic installer metadata"]
    fn differential_stage_os_installer_legacy_and_shared_support() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let app = source.join("Install macOS Fixture.app");
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::create_dir_all(app.join("Contents/SharedSupport")).unwrap();
        std::fs::write(
            app.join("Contents/Resources/startosinstall"),
            "#!/bin/sh\nexit 99\n",
        )
        .unwrap();
        Value::Dictionary(Dictionary::from_iter([
            (
                "CFBundleName",
                Value::String("Install macOS Fixture".into()),
            ),
            ("CFBundleShortVersionString", "17.0".into()),
            ("LSMinimumSystemVersion", "12.3".into()),
        ]))
        .to_file_xml(app.join("Contents/Info.plist"))
        .unwrap();
        Value::Dictionary(Dictionary::from_iter([(
            "System Image Info",
            Value::Dictionary(Dictionary::from_iter([(
                "version",
                Value::String("12.6".into()),
            )])),
        )]))
        .to_file_xml(app.join("Contents/SharedSupport/InstallInfo.plist"))
        .unwrap();
        let legacy = temp.path().join("legacy.dmg");
        command(
            "/usr/bin/hdiutil",
            &[
                "create".as_ref(),
                "-srcfolder".as_ref(),
                source.as_os_str(),
                "-format".as_ref(),
                "UDZO".as_ref(),
                legacy.as_os_str(),
            ],
        )
        .unwrap();
        assert_eq!(
            normalize(generate(Some(&legacy), &Options::default()).unwrap()),
            normalize(reference(Some(&legacy), &[]))
        );
        let copy: Vec<String> = ["--installer-type", "copy_from_dmg"]
            .iter()
            .map(|s| (*s).into())
            .collect();
        assert_eq!(
            normalize(generate(Some(&legacy), &Options::parse(&copy).unwrap()).unwrap()),
            normalize(reference(Some(&legacy), &copy))
        );
        std::fs::remove_file(app.join("Contents/SharedSupport/InstallInfo.plist")).unwrap();
        let assets = temp
            .path()
            .join("assets/com_apple_MobileAsset_MacSoftwareUpdate");
        std::fs::create_dir_all(&assets).unwrap();
        Value::Dictionary(Dictionary::from_iter([(
            "Assets",
            Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
                ("OSVersion", Value::String("15.2".into())),
                (
                    "SupportedDeviceModels",
                    Value::Array(vec!["Mac-EXAMPLE".into(), "J123AP".into()]),
                ),
            ]))]),
        )]))
        .to_file_xml(assets.join("com_apple_MobileAsset_MacSoftwareUpdate.xml"))
        .unwrap();
        command(
            "/usr/bin/hdiutil",
            &[
                "create".as_ref(),
                "-srcfolder".as_ref(),
                assets.parent().unwrap().as_os_str(),
                "-format".as_ref(),
                "UDZO".as_ref(),
                app.join("Contents/SharedSupport/SharedSupport.dmg")
                    .as_os_str(),
            ],
        )
        .unwrap();
        let modern = temp.path().join("modern.dmg");
        command(
            "/usr/bin/hdiutil",
            &[
                "create".as_ref(),
                "-srcfolder".as_ref(),
                source.as_os_str(),
                "-format".as_ref(),
                "UDZO".as_ref(),
                modern.as_os_str(),
            ],
        )
        .unwrap();
        let args: Vec<String> = ["--installer-type", "stage_os_installer"]
            .iter()
            .map(|s| (*s).into())
            .collect();
        assert_eq!(
            normalize(generate(Some(&modern), &Options::parse(&args).unwrap()).unwrap()),
            normalize(reference(Some(&modern), &args))
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires pinned Munki reference and native legacy package tools; does not install packages"]
    fn differential_bundle_package_metadata_or_native_tool_rejection() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("Legacy-2.0.pkg");
        std::fs::create_dir_all(package.join("Contents/Resources")).unwrap();
        let payload = temp.path().join("payload");
        std::fs::create_dir(&payload).unwrap();
        std::fs::write(payload.join("fixture"), "payload").unwrap();
        Value::Dictionary(Dictionary::from_iter([
            (
                "CFBundleIdentifier",
                Value::String("org.autopkg.legacy.fixture".into()),
            ),
            ("CFBundleName", "Legacy".into()),
            ("CFBundleShortVersionString", "2.0".into()),
            ("CFBundleVersion", "2.0".into()),
            ("IFPkgFlagInstalledSize", Value::Integer(1.into())),
            ("IFPkgFlagAuthorizationAction", "RootAuthorization".into()),
            ("IFPkgFlagDefaultLocation", "/".into()),
            ("IFPkgFlagRestartAction", "NoRestart".into()),
            ("IFPkgFormatVersion", Value::Real(0.1)),
        ]))
        .to_file_xml(package.join("Contents/Info.plist"))
        .unwrap();
        std::fs::write(package.join("Contents/PkgInfo"), "pmkrpkg1").unwrap();
        command(
            "/usr/bin/mkbom",
            &[
                payload.as_os_str(),
                package.join("Contents/Archive.bom").as_os_str(),
            ],
        )
        .unwrap();
        command(
            "/usr/bin/ditto",
            &[
                "-c".as_ref(),
                "-z".as_ref(),
                payload.as_os_str(),
                package.join("Contents/Archive.pax.gz").as_os_str(),
            ],
        )
        .unwrap();
        let actual = generate(Some(&package), &Options::default());
        let reference_output = Command::new("/usr/local/munki/makepkginfo")
            .arg(&package)
            .output()
            .unwrap();
        if reference_output.status.success() {
            let expected = Value::from_reader(std::io::Cursor::new(reference_output.stdout))
                .unwrap()
                .into_dictionary()
                .unwrap();
            assert_eq!(normalize(actual.unwrap()), normalize(expected));
        } else {
            // Newer macOS installer tools may reject legacy bundle packages.
            // Match that failure rather than bypassing the native restart query.
            assert!(actual.is_err());
        }
    }
}
