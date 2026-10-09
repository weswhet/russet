//! Package tools that the package processors share: running Apple's tools,
//! Russet's native replacements for them, and glob logging.
use super::Result;
use autopkg_platform::processor_output as output;
use std::{path::PathBuf, process::Command};

/// The error for a package processor on a platform without the tools it
/// needs.
pub(crate) fn unsupported(name: &str) -> String {
    format!("{name} is only supported on macOS and Linux; pkgutil and xar are unavailable on this platform")
}
pub(crate) fn run(binary: &str, args: &[&str]) -> Result<()> {
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
/// Russet's replacements for `pkgutil`, `xar`, `ditto`, and `aa` on
/// packages, used on Linux and with `RUSSET_NATIVE`.
#[cfg(unix)]
pub(crate) mod native {
    use super::Result;
    use russet_fs::Limits;
    use std::path::Path;

    pub(crate) fn flatten(source: &str, destination: &str) -> Result<()> {
        russet_pkgutil::flatten(Path::new(source), Path::new(destination))
            .map_err(|e| format!("pkgutil --flatten failed: {e}"))
    }

    pub(crate) fn expand(source: &str, destination: &str) -> Result<()> {
        russet_pkgutil::expand(Path::new(source), Path::new(destination), Limits::default())
            .map_err(|e| format!("pkgutil --expand failed: {e}"))
    }

    /// `xar -x --exclude Payload`: the pattern is a regular expression
    /// matched against each path, so any path containing `Payload` is
    /// skipped.
    pub(crate) fn extract_without_payload(source: &str, destination: &str) -> Result<()> {
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
    pub(crate) fn extract_apple_archive(source: &str, destination: &str) -> Result<()> {
        russet_aa::extract(Path::new(source), Path::new(destination), Limits::default())
            .map(|_| ())
            .map_err(|e| format!("Unpacking {source} failed: {e}"))
    }
    pub(crate) fn extract_payload(source: &str, destination: &str) -> Result<()> {
        russet_ditto::extract_cpio(Path::new(source), Path::new(destination), Limits::default())
            .map(|_| ())
            .map_err(|e| format!("Unpacking {source} failed: {e}"))
    }
}

#[cfg(not(unix))]
pub(crate) mod native {
    use super::Result;
    fn unavailable() -> Result<()> {
        Err("Native package tools require macOS or Linux".into())
    }
    pub(crate) fn flatten(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(crate) fn expand(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(crate) fn extract_without_payload(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(crate) fn extract_payload(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
    pub(crate) fn extract_apple_archive(_: &str, _: &str) -> Result<()> {
        unavailable()
    }
}
pub(crate) fn log_glob(key: &str, pattern: &str, paths: &[PathBuf]) {
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
    #[cfg(unix)]
    use super::*;
    use crate::tests::{env, Temp};
    #[cfg(target_os = "macos")]
    use plist::Dictionary;
    use std::fs;
    #[cfg(unix)]
    use std::path::Path;
    use xmltree::Element;
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
