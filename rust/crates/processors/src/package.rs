use super::{io, portable_path, read_dict, remove, string, truth, Result};
use autopkg_platform::backend::{select, Backend, Tool};
use autopkg_platform::processor_output as output;
use plist::Dictionary;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use xmltree::{Element, XMLNode};
fn find_template(env: &Dictionary) -> Result<PathBuf> {
    let path = Path::new(string(env, "template_path")?);
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    if path.is_relative() {
        let mut directories = Vec::new();
        if let Some(directory) = env.get("RECIPE_DIR").and_then(plist::Value::as_string) {
            directories.push(PathBuf::from(directory));
        }
        if let Some(parents) = env.get("PARENT_RECIPES").and_then(plist::Value::as_array) {
            for parent in parents {
                let path = Path::new(
                    parent
                        .as_string()
                        .ok_or("PARENT_RECIPES must contain strings")?,
                );
                if let Some(dir) = path.parent() {
                    directories.push(dir.to_path_buf());
                }
            }
        }
        for directory in directories {
            let candidate = directory.join(path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Err(format!("Can't find {}", path.display()))
}
fn template(path: &Path) -> Result<Element> {
    if path.extension().is_some_and(|e| e == "plist") {
        let data = read_dict(path)?;
        let mut element = Element::new("pkg-info");
        element
            .attributes
            .insert("format-version".into(), "2".into());
        for (source, target) in [
            ("IFPkgFlagDefaultLocation", "install-location"),
            ("CFBundleShortVersionString", "version"),
            ("CFBundleIdentifier", "identifier"),
        ] {
            if let Some(value) = data.get(source) {
                element.attributes.insert(
                    target.into(),
                    value
                        .as_string()
                        .ok_or_else(|| format!("{source} must be a string"))?
                        .into(),
                );
            }
        }
        if let Some(auth) = data.get("IFPkgFlagAuthorizationAction") {
            element.attributes.insert(
                "auth".into(),
                if auth.as_string() == Some("RootAuthorization") {
                    "root"
                } else {
                    "none"
                }
                .into(),
            );
        }
        if let Some(action) = data.get("IFPkgFlagRestartAction") {
            let action = match action.as_string() {
                Some("RecommendRestart" | "RequireRestart") => "restart",
                Some("RequireLogout") => "logout",
                Some("RequireShutdown") => "shutdown",
                Some("None") => "none",
                _ => {
                    output(1, format!("WARNING: Unrecognized IFPkgFlagRestartAction '{}' in template; treating as 'none'.", plist::python_str(action)));
                    "none"
                }
            };
            element
                .attributes
                .insert("postinstall-action".into(), action.into());
        }
        element
            .children
            .push(XMLNode::Element(Element::new("payload")));
        Ok(element)
    } else {
        Element::parse(io(fs::File::open(path))?)
            .map_err(|e| format!("Malformed PackageInfo template {}: {e}", path.display()))
    }
}
fn size(root: &Path) -> Result<(u64, u64)> {
    // Match os.walk: nonexistent roots produce zero counts and directory symlinks
    // are listed as directories but never visited or included in payload size.
    if !root.is_dir() {
        return Ok((0, 0));
    }
    let mut blocks = 0;
    let mut files = 1;
    for entry in io(fs::read_dir(root))? {
        let path = io(entry)?.path();
        let metadata = io(fs::symlink_metadata(&path))?;
        if path.is_dir() {
            if !metadata.file_type().is_symlink() {
                let (b, f) = size(&path)?;
                blocks += b;
                files += f;
            }
        } else {
            blocks += metadata.len().div_ceil(4096);
            files += 1;
        }
    }
    Ok((blocks, files))
}
fn create_info(env: &Dictionary) -> Result<()> {
    let kind = string(env, "pkgtype")?;
    if kind != "flat" && kind != "bundle" {
        return Err(format!("Unknown pkgtype {kind}"));
    }
    let mut info = template(&find_template(env)?)?;
    if kind == "bundle" {
        return Err("Bundle package creation no longer supported!".into());
    }
    if info.name != "pkg-info" || info.namespace.is_some() {
        return Err("PackageInfo root should be pkg-info".into());
    }
    info.attributes
        .insert("version".into(), string(env, "version")?.into());
    if info.get_child("payload").is_none() {
        info.children
            .push(XMLNode::Element(Element::new("payload")));
    }
    let payload = info.get_mut_child("payload").unwrap();
    let (blocks, files) = size(Path::new(string(env, "pkgroot")?))?;
    payload
        .attributes
        .insert("installKBytes".into(), blocks.to_string());
    payload
        .attributes
        .insert("numberOfFiles".into(), files.to_string());
    info.write_with_config(
        io(fs::File::create(string(env, "infofile")?))?,
        xmltree::EmitterConfig::new().write_document_declaration(false),
    )
    .map_err(|e| e.to_string())
}
fn run(binary: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|e| format!("{binary} execution failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{binary} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
pub(super) fn execute(name: &str, env: &mut Dictionary) -> Result<()> {
    if name == "PkgInfoCreator" {
        return create_info(env);
    }
    let unsupported = || {
        format!("{name} is only supported on macOS and Linux; pkgutil and xar are unavailable on this platform")
    };
    if name == "FlatPkgPacker" {
        let (source, destination) = (
            string(env, "source_flatpkg_dir")?,
            string(env, "destination_pkg")?,
        );
        match select(Tool::Pkgutil) {
            Backend::Apple => run("/usr/sbin/pkgutil", &["--flatten", source, destination])?,
            Backend::Native => native::flatten(source, destination)?,
            Backend::Unsupported => return Err(unsupported()),
        }
        output(
            1,
            format!(
                "Flattened {} to {}",
                string(env, "source_flatpkg_dir")?,
                string(env, "destination_pkg")?
            ),
        );
        return Ok(());
    }
    let source = string(env, "flat_pkg_path")?;
    portable_path(source)?;
    let destination = string(env, "destination_path")?;
    let path = Path::new(destination);
    if !path.exists() {
        io(fs::create_dir_all(path))?;
    } else if truth(env.get("purge_destination")) {
        for entry in io(fs::read_dir(path))? {
            remove(&io(entry)?.path())?;
        }
    }
    if truth(env.get("skip_payload")) {
        match select(Tool::Xar) {
            Backend::Apple => run(
                "/usr/bin/xar",
                &[
                    "-x",
                    "-C",
                    destination,
                    "-f",
                    source,
                    "--exclude",
                    "Payload",
                ],
            ),
            Backend::Native => native::extract_without_payload(source, destination),
            Backend::Unsupported => Err(unsupported()),
        }
    } else {
        if path.exists() {
            io(fs::remove_dir_all(path))?;
        }
        match select(Tool::Pkgutil) {
            Backend::Apple => run("/usr/sbin/pkgutil", &["--expand", source, destination]),
            Backend::Native => native::expand(source, destination),
            Backend::Unsupported => Err(unsupported()),
        }
    }?;
    output(1, format!("Unpacked {source} to {destination}"));
    Ok(())
}
pub(super) fn copy(env: &mut Dictionary) -> Result<()> {
    env.remove("pkg_copier_summary_result");
    let source = string(env, "source_pkg")?;
    super::portable_path(source)?;
    let paths = super::python_glob::paths_with_recursion(source, false)?;
    let source = paths
        .first()
        .ok_or("Error processing source_pkg with glob")?;
    log_glob("source_pkg", string(env, "source_pkg")?, &paths);
    if !source
        .extension()
        .is_some_and(|e| e == "pkg" || e == "mpkg")
    {
        return Err(format!(
            "Source does not appear to be a package based on its filename: '{}'",
            source.display()
        ));
    }
    let destination = match env
        .get("pkg_path")
        .and_then(plist::Value::as_string)
        .filter(|s| !s.is_empty())
    {
        Some(path) => PathBuf::from(path),
        None => Path::new(string(env, "RECIPE_CACHE_DIR")?).join(source.file_name().unwrap()),
    };
    // Keep Copier's temporary input keys out of the caller's environment.
    let mut inputs = Dictionary::new();
    inputs.insert(
        "source_path".into(),
        source.to_string_lossy().into_owned().into(),
    );
    inputs.insert(
        "destination_path".into(),
        destination.to_string_lossy().into_owned().into(),
    );
    inputs.insert("overwrite".into(), true.into());
    super::execute("Copier", &mut inputs)?;
    output(
        1,
        format!("Copied {} to {}", source.display(), destination.display()),
    );
    let destination = destination.to_string_lossy().into_owned();
    env.insert("pkg_path".into(), destination.clone().into());
    let mut data = Dictionary::new();
    data.insert("pkg_path".into(), destination.into());
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        "The following packages were copied:".into(),
    );
    summary.insert("data".into(), data.into());
    env.insert("pkg_copier_summary_result".into(), summary.into());
    Ok(())
}

fn prepare_destination(env: &Dictionary) -> Result<()> {
    let path = Path::new(string(env, "destination_path")?);
    if !path.exists() {
        io(fs::create_dir_all(path))?;
    } else if truth(env.get("purge_destination")) {
        for entry in io(fs::read_dir(path))? {
            remove(&io(entry)?.path())?;
        }
    }
    Ok(())
}
pub(super) fn unpack_payload(env: &Dictionary) -> Result<()> {
    let backend = select(Tool::Ditto);
    if backend == Backend::Unsupported {
        return Err(
            "Package payload extraction is only supported on macOS and Linux; ditto and aa are unavailable"
                .into(),
        );
    }
    prepare_destination(env)?;
    let source = string(env, "pkg_payload_path")?;
    let destination = string(env, "destination_path")?;
    if backend == Backend::Native {
        native::extract_payload(source, destination)?;
    } else {
        match run("/usr/bin/ditto", &["-x", "-z", source, destination]) {
            Ok(()) => Ok(()),
            Err(ditto_error) if Path::new("/usr/bin/aa").exists() => {
                run("/usr/bin/aa", &["extract", "-i", source, "-d", destination])
                    .map_err(|error| format!("{ditto_error}; {error}"))
            }
            Err(error) => Err(error),
        }?;
    }
    output(1, format!("Unpacked {source} to {destination}"));
    Ok(())
}
pub(super) fn extract_bundle(env: &Dictionary) -> Result<()> {
    let backend = select(Tool::Ditto);
    if backend == Backend::Unsupported {
        return Err(
            "Bundle package extraction is only supported on macOS and Linux; ditto is unavailable"
                .into(),
        );
    }
    let package = Path::new(string(env, "pkg_path")?);
    let info = package.join("Contents/Info.plist");
    let archive = package.join("Contents/Archive.pax.gz");
    if !info.exists() {
        return Err("Info.plist not found in pkg".into());
    }
    if !archive.exists() {
        return Err("Archive.pax.gz not found in pkg".into());
    }
    let info = read_dict(&info)?;
    let location = info
        .get("IFPkgFlagDefaultLocation")
        .map(|v| {
            v.as_string()
                .ok_or("IFPkgFlagDefaultLocation must be a string")
        })
        .transpose()?
        .unwrap_or("/");
    let mut relative = PathBuf::new();
    for part in Path::new(location.trim_start_matches('/')).components() {
        match part {
            std::path::Component::Normal(p) => relative.push(p),
            std::path::Component::CurDir => (),
            std::path::Component::ParentDir => {
                if !relative.pop() {
                    return Err(format!(
                        "IFPkgFlagDefaultLocation {location:?} resolves outside extract_root"
                    ));
                }
            }
            _ => return Err("IFPkgFlagDefaultLocation resolves outside extract_root".into()),
        }
    }
    let root = Path::new(string(env, "extract_root")?);
    let destination = root.join(relative);
    // Validate existing symlink ancestors before replacing the root.
    if root.exists() {
        let realroot = io(root.canonicalize())?;
        for ancestor in destination.ancestors() {
            if ancestor.exists() {
                if !io(ancestor.canonicalize())?.starts_with(&realroot) {
                    return Err("IFPkgFlagDefaultLocation resolves outside extract_root".into());
                }
                break;
            }
        }
        io(fs::remove_dir_all(root))?;
    }
    io(fs::create_dir_all(&destination))?;
    super::mode(&destination, "755")?;
    let archive = archive.to_str().ok_or("Invalid package path")?;
    let destination = destination.to_str().ok_or("Invalid extraction path")?;
    if backend == Backend::Native {
        return native::extract_payload(archive, destination);
    }
    run("/usr/bin/ditto", &["-x", "-z", archive, destination])
}

/// Russet's replacements for `pkgutil`, `xar`, `ditto`, and `aa` on
/// packages, used on Linux and with `RUSSET_NATIVE`.
#[cfg(unix)]
mod native {
    use super::Result;
    use russet_fs::Limits;
    use std::path::Path;

    pub(super) fn flatten(source: &str, destination: &str) -> Result<()> {
        russet_pkgutil::flatten(Path::new(source), Path::new(destination))
            .map_err(|e| format!("pkgutil --flatten failed: {e}"))
    }

    pub(super) fn expand(source: &str, destination: &str) -> Result<()> {
        russet_pkgutil::expand(Path::new(source), Path::new(destination), Limits::default())
            .map_err(|e| format!("pkgutil --expand failed: {e}"))
    }

    /// `xar -x --exclude Payload`: the pattern is a regular expression
    /// matched against each path, so any path containing `Payload` is
    /// skipped.
    pub(super) fn extract_without_payload(source: &str, destination: &str) -> Result<()> {
        let mut archive =
            russet_xar::Archive::open(Path::new(source)).map_err(|e| format!("xar failed: {e}"))?;
        archive
            .extract(Path::new(destination), Limits::default(), |entry| {
                entry.path.to_string_lossy().contains("Payload")
            })
            .map(|_| ())
            .map_err(|e| format!("xar failed: {e}"))
    }

    /// `ditto -x -z`, or `aa extract` for pbzx payloads.
    pub(super) fn extract_payload(source: &str, destination: &str) -> Result<()> {
        russet_ditto::extract_cpio(Path::new(source), Path::new(destination), Limits::default())
            .map(|_| ())
            .map_err(|e| format!("Unpacking {source} failed: {e}"))
    }
}

#[cfg(not(unix))]
mod native {
    use super::Result;
    fn unavailable() -> Result<()> {
        Err("Native package tools require macOS or Linux".into())
    }
    pub(super) fn flatten(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(super) fn expand(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(super) fn extract_without_payload(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(super) fn extract_payload(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
}
pub(super) fn log_glob(key: &str, pattern: &str, paths: &[PathBuf]) {
    if paths.len() > 1 {
        output(
            1,
            format!("WARNING: Multiple paths match '{key}' glob '{pattern}':"),
        );
        for path in paths {
            output(1, format!("  - {}", path.display()));
        }
    }
    if pattern.contains(['*', '?', '[', ']', '!']) {
        if let Some(path) = paths.first() {
            output(
                1,
                format!(
                    "Using path '{}' matched from globbed '{pattern}'.",
                    path.display()
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{env, Temp};
    /// Builds a component package the way `pkgbuild` lays it out, then runs
    /// the native package processors over it. This is the Linux path; on
    /// macOS, comparisons with Apple's tools live in the native crates.
    #[cfg(unix)]
    #[test]
    fn native_package_processors() {
        use russet_xar::{Builder, Content, Encoding};
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let t = Temp::new();
        let root = Path::new(&t.path("root")).join("Applications/Tool.app/Contents/MacOS");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Tool"), "binary").unwrap();
        fs::set_permissions(root.join("Tool"), fs::Permissions::from_mode(0o755)).unwrap();
        let archive = |source: &str, out: &str| {
            let encoder = flate2::write::GzEncoder::new(
                fs::File::create(t.path(out)).unwrap(),
                flate2::Compression::default(),
            );
            russet_ditto::write_tree(Path::new(&t.path(source)), encoder, |_, m| {
                russet_ditto::Header {
                    mode: m.mode(),
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    ino: 0,
                    nlink: m.nlink() as u32,
                }
            })
            .unwrap()
            .finish()
            .unwrap();
        };
        archive("root", "payload.cpgz");
        fs::create_dir(t.path("scripts")).unwrap();
        fs::write(t.path("scripts/postinstall"), "#!/bin/sh\n").unwrap();
        archive("scripts", "scripts.cpgz");
        let mut builder = Builder::new();
        let file = |name: &str| Content::Path(t.path(name).into());
        builder
            .add_file(
                Path::new("PackageInfo"),
                0o644,
                Content::Bytes(b"<pkg-info/>".to_vec()),
                Encoding::Bzip2,
            )
            .unwrap();
        builder
            .add_file(
                Path::new("Payload"),
                0o644,
                file("payload.cpgz"),
                Encoding::None,
            )
            .unwrap();
        builder
            .add_file(
                Path::new("Scripts"),
                0o644,
                file("scripts.cpgz"),
                Encoding::None,
            )
            .unwrap();
        let package = t.path("Tool.pkg");
        builder.write(Path::new(&package)).unwrap();

        native::expand(&package, &t.path("expanded")).unwrap();
        assert!(Path::new(&t.path("expanded/Scripts/postinstall")).is_file());
        assert!(Path::new(&t.path("expanded/Payload")).is_file());
        native::extract_payload(&t.path("expanded/Payload"), &t.path("payload")).unwrap();
        let tool = Path::new(&t.path("payload")).join("Applications/Tool.app/Contents/MacOS/Tool");
        assert_eq!(
            fs::metadata(&tool).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::create_dir(t.path("skip")).unwrap();
        native::extract_without_payload(&package, &t.path("skip")).unwrap();
        assert!(!Path::new(&t.path("skip/Payload")).exists());
        assert!(Path::new(&t.path("skip/Scripts")).is_file());
        native::flatten(&t.path("expanded"), &t.path("again.pkg")).unwrap();
        native::expand(&t.path("again.pkg"), &t.path("again")).unwrap();
        assert_eq!(
            fs::read(t.path("again/Scripts/postinstall")).unwrap(),
            b"#!/bin/sh\n"
        );
    }
    #[test]
    fn metadata_preserves_template_and_reference_block_count() {
        let t = Temp::new();
        let root = t.0.join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("large"), vec![0; 4097]).unwrap();
        fs::write(root.join("empty"), []).unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        let template = t.path("template");
        fs::write(&template, "<pkg-info identifier='test'><scripts><postinstall file='postinstall'/></scripts></pkg-info>").unwrap();
        let output = t.path("PackageInfo");
        let mut e = env(&[
            ("template_path", &template),
            ("version", "2.0"),
            ("pkgroot", root.to_str().unwrap()),
            ("pkgtype", "flat"),
            ("infofile", &output),
        ]);
        crate::execute("PkgInfoCreator", &mut e).unwrap();
        let info = Element::parse(fs::File::open(&output).unwrap()).unwrap();
        assert_eq!(info.attributes["version"], "2.0");
        assert!(info.get_child("scripts").is_some());
        let payload = info.get_child("payload").unwrap();
        assert_eq!(payload.attributes["installKBytes"], "2");
        assert_eq!(payload.attributes["numberOfFiles"], "4");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_package_roundtrip_and_skip_payload() {
        let t = Temp::new();
        let root = t.0.join("root");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("hello"), "fixture").unwrap();
        let package = t.path("fixture.pkg");
        run(
            "/usr/bin/pkgbuild",
            &[
                "--root",
                root.to_str().unwrap(),
                "--identifier",
                "org.autopkg.rust.fixture",
                "--version",
                "1.0",
                &package,
            ],
        )
        .unwrap();
        let expanded = t.path("expanded");
        let mut e = env(&[("flat_pkg_path", &package), ("destination_path", &expanded)]);
        crate::execute("FlatPkgUnpacker", &mut e).unwrap();
        assert!(Path::new(&expanded).join("PackageInfo").exists());
        assert!(Path::new(&expanded).join("Payload").exists());
        let mut payload_env = env(&[
            ("pkg_payload_path", &format!("{expanded}/Payload")),
            ("destination_path", &t.path("payload")),
        ]);
        crate::execute("PkgPayloadUnpacker", &mut payload_env).unwrap();
        assert_eq!(fs::read(t.path("payload/hello")).unwrap(), b"fixture");
        let bundle = t.0.join("Bundle.pkg/Contents");
        fs::create_dir_all(&bundle).unwrap();
        fs::copy(
            Path::new(&expanded).join("Payload"),
            bundle.join("Archive.pax.gz"),
        )
        .unwrap();
        let mut info = Dictionary::new();
        info.insert("IFPkgFlagDefaultLocation".into(), "/Applications".into());
        plist::Value::Dictionary(info)
            .to_file_xml(bundle.join("Info.plist"))
            .unwrap();
        let mut bundle_env = env(&[
            ("pkg_path", &t.path("Bundle.pkg")),
            ("extract_root", &t.path("bundle-output")),
        ]);
        crate::execute("PkgExtractor", &mut bundle_env).unwrap();
        assert_eq!(
            fs::read(t.path("bundle-output/Applications/hello")).unwrap(),
            b"fixture"
        );

        let repacked = t.path("repacked.pkg");
        e.insert("source_flatpkg_dir".into(), expanded.into());
        e.insert("destination_pkg".into(), repacked.clone().into());
        crate::execute("FlatPkgPacker", &mut e).unwrap();
        e.insert("flat_pkg_path".into(), repacked.into());
        e.insert("destination_path".into(), t.path("metadata").into());
        e.insert("skip_payload".into(), true.into());
        crate::execute("FlatPkgUnpacker", &mut e).unwrap();
        assert!(t.0.join("metadata/PackageInfo").exists());
        assert!(!t.0.join("metadata/Payload").exists());
    }
}
