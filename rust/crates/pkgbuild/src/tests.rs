use crate::{analyze, build, collect, NodeKind, Options};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn sample_root(root: &Path) {
    let app = root.join("Applications/Tool.app/Contents");
    fs::create_dir_all(app.join("MacOS")).unwrap();
    fs::write(
        app.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.example.tool</string><key>CFBundleShortVersionString</key><string>1.2</string><key>CFBundleVersion</key><string>12</string></dict></plist>"#,
    )
    .unwrap();
    fs::write(app.join("MacOS/Tool"), "binary").unwrap();
    fs::set_permissions(app.join("MacOS/Tool"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("MacOS/Tool", app.join("Link")).unwrap();
    fs::write(app.join(".DS_Store"), "junk").unwrap();
}

fn nodes_as_root(root: &Path) -> Vec<crate::Node> {
    let mut nodes = collect(root, &[]).unwrap();
    for node in &mut nodes {
        node.uid = 0;
        node.gid = if node.path.is_empty() || node.path == "Applications" {
            80
        } else {
            0
        };
        if node.path.is_empty() {
            node.mode = 0o1775;
        }
    }
    nodes
}

#[test]
fn builds_a_component_package() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    sample_root(&root);
    let components = analyze(&root).unwrap();
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].path, "Applications/Tool.app");
    assert!(components[0].strict_identifier);
    let nodes = nodes_as_root(&root);
    assert!(nodes.iter().all(|n| !n.path.ends_with(".DS_Store")));
    let scripts = temp.path().join("scripts");
    fs::create_dir(&scripts).unwrap();
    fs::write(scripts.join("postinstall"), "#!/bin/sh\nexit 0\n").unwrap();
    let out = temp.path().join("Tool.pkg");
    let options = Options {
        identifier: "com.example.tool",
        version: "1.2",
        install_location: Some("/"),
        min_os_version: None,
        scripts: Some(&scripts),
        info_template: None,
        components: &components,
    };
    build(&nodes, &options, &out).unwrap();

    let mut archive = russet_xar::Archive::open(&out).unwrap();
    let names: Vec<_> = archive
        .entries()
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["Bom", "Payload", "Scripts", "PackageInfo"]);
    let info = String::from_utf8(archive.read("PackageInfo", 1 << 20).unwrap()).unwrap();
    let document = roxmltree::Document::parse(&info).unwrap();
    let root_element = document.root_element();
    assert_eq!(
        root_element.attribute("identifier"),
        Some("com.example.tool")
    );
    assert_eq!(root_element.attribute("install-location"), Some("/"));
    assert!(info.contains(r#"<bundle path="./Applications/Tool.app" id="com.example.tool" CFBundleShortVersionString="1.2" CFBundleVersion="12"/>"#));
    assert!(info.contains(r#"<postinstall file="./postinstall" timeout="600"/>"#));

    let bom = russet_mkbom::read(&archive.read("Bom", 1 << 20).unwrap()).unwrap();
    // Every BOM entry counts, including `._name` members for any extended
    // attributes the host added to the files.
    assert_eq!(
        document
            .descendants()
            .find(|n| n.has_tag_name("payload"))
            .unwrap()
            .attribute("numberOfFiles"),
        Some(bom.len().to_string().as_str())
    );
    let listing: Vec<String> = bom.iter().map(|e| e.lsbom_line()).collect();
    assert_eq!(listing[0], ".\t41775\t0/80");
    assert!(listing
        .iter()
        .any(|l| l.starts_with("./Applications/Tool.app/Contents/MacOS/Tool\t100755\t0/0\t6\t")));
    assert!(listing
        .iter()
        .any(|l| l.starts_with("./Applications/Tool.app/Contents/Link\t120")));

    let expanded = temp.path().join("expanded");
    fs::create_dir(&expanded).unwrap();
    archive
        .extract(&expanded, russet_fs::Limits::default(), |_| false)
        .unwrap();
    let payload = temp.path().join("payload");
    russet_ditto::extract_cpio(
        &expanded.join("Payload"),
        &payload,
        russet_fs::Limits::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(payload.join("Applications/Tool.app/Contents/MacOS/Tool")).unwrap(),
        b"binary"
    );
    assert_eq!(
        fs::read_link(payload.join("Applications/Tool.app/Contents/Link")).unwrap(),
        Path::new("MacOS/Tool")
    );
    assert!(nodes.iter().any(|n| matches!(n.kind, NodeKind::Symlink(_))));
}

/// Extended attributes, including ones too large for ext4 that an
/// extraction kept in its sidecar, travel in `._name` members and come back
/// when the payload is extracted.
#[test]
fn keeps_extended_attributes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let large = vec![7u8; 64 << 10];
    fs::create_dir(&root).unwrap();
    let mut writer = russet_fs::TreeWriter::open(&root, russet_fs::Limits::default()).unwrap();
    writer.create_dir(Path::new("bin"), Some(0o755)).unwrap();
    writer
        .write_file(Path::new("bin/tool"), &b"#!/bin/sh\n"[..], 0o755)
        .unwrap();
    writer
        .set_xattr(Path::new("bin/tool"), "com.apple.cs.CodeDirectory", b"cd")
        .unwrap();
    writer
        .set_xattr(Path::new("bin/tool"), "com.apple.cs.CodeSignature", &large)
        .unwrap();
    assert!(writer.finish().unwrap().is_empty());

    let nodes = nodes_as_root(&root);
    assert!(nodes.iter().all(|n| !n.path.contains(russet_fs::SIDECAR)));
    let out = temp.path().join("tool.pkg");
    let options = Options {
        identifier: "com.example.tool",
        version: "1",
        install_location: Some("/"),
        min_os_version: None,
        scripts: None,
        info_template: None,
        components: &[],
    };
    build(&nodes, &options, &out).unwrap();
    let mut archive = russet_xar::Archive::open(&out).unwrap();
    let bom = russet_mkbom::read(&archive.read("Bom", 1 << 20).unwrap()).unwrap();
    let listing: Vec<String> = bom.iter().map(|e| e.lsbom_line()).collect();
    assert!(
        listing.contains(&"./bin/._tool\t100755\t0/0\t0\t0".to_owned()),
        "{listing:?}"
    );

    let expanded = temp.path().join("expanded");
    fs::create_dir(&expanded).unwrap();
    archive
        .extract(&expanded, russet_fs::Limits::default(), |_| false)
        .unwrap();
    let payload = temp.path().join("payload");
    russet_ditto::extract_cpio(
        &expanded.join("Payload"),
        &payload,
        russet_fs::Limits::default(),
    )
    .unwrap();
    let tool = payload.join("bin/tool");
    assert_eq!(
        russet_fs::get_xattr(&tool, "com.apple.cs.CodeDirectory")
            .unwrap()
            .as_deref(),
        Some(&b"cd"[..])
    );
    assert_eq!(
        russet_fs::get_xattr(&tool, "com.apple.cs.CodeSignature").unwrap(),
        Some(large)
    );
    assert!(!payload.join("bin/._tool").exists());
}

/// Compares with Apple's `pkgbuild` on the same root, and checks that
/// Apple's tools accept the native package.
#[cfg(target_os = "macos")]
#[test]
fn matches_apple_pkgbuild() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    sample_root(&root);
    let t = |p: &str| temp.path().join(p);
    let run = |c: &mut Command| {
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{c:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let components = analyze(&root).unwrap();
    // Apple's pkgbuild records the disk's owners with --ownership preserve,
    // so the native build uses the disk's owners too.
    let nodes = collect(&root, &[]).unwrap();
    build(
        &nodes,
        &Options {
            identifier: "com.example.tool",
            version: "1.2",
            install_location: None,
            min_os_version: None,
            scripts: None,
            info_template: None,
            components: &components,
        },
        &t("native.pkg"),
    )
    .unwrap();
    run(Command::new("/usr/bin/pkgbuild")
        .args([
            "--quiet",
            "--ownership",
            "preserve",
            "--identifier",
            "com.example.tool",
            "--version",
            "1.2",
            "--root",
        ])
        .arg(&root)
        .arg(t("apple.pkg")));
    let expand = |name: &str| {
        run(Command::new("/usr/sbin/pkgutil")
            .arg("--expand")
            .arg(t(&format!("{name}.pkg")))
            .arg(t(name)));
        // Ignore AppleDouble entries that come from the host's own attributes.
        run(Command::new("/usr/bin/lsbom").arg(t(name).join("Bom")))
            .lines()
            .filter(|l| !l.contains("/._"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(expand("native"), expand("apple"));
    // installer parses the native package's metadata.
    run(Command::new("/usr/sbin/installer")
        .args(["-pkginfo", "-pkg"])
        .arg(t("native.pkg")));
    let restart = run(Command::new("/usr/sbin/installer")
        .args(["-query", "RestartAction", "-pkg"])
        .arg(t("native.pkg")));
    assert_eq!(restart.trim(), "None");
    // ditto unpacks the native payload into the same tree.
    let unpack = |name: &str| {
        let dest = t(&format!("{name}-payload"));
        run(Command::new("/usr/bin/ditto")
            .args(["-x", "-z"])
            .arg(t(name).join("Payload"))
            .arg(&dest));
        russet_fs::manifest(&dest)
            .unwrap()
            .into_iter()
            .filter(|e| !e.path.contains("._"))
            .map(|mut e| {
                e.xattrs.clear();
                e
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(unpack("native"), unpack("apple"));
}

/// Installs a natively built package with Apple's `installer` and checks the
/// receipt and the installed tree. Needs passwordless `sudo`, so it's
/// ignored locally; macOS CI runs it with
/// `cargo test -p russet-pkgbuild -- --ignored installs_with_apple_installer`.
/// With `RUSSET_KEEP_TEST_PACKAGE=DIR`, it copies the package to `DIR` and
/// stops before installing, for inspection without `sudo`.
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn installs_with_apple_installer() {
    use std::os::unix::fs::MetadataExt;
    use std::process::Command;
    let sudo = |args: &[&str]| {
        let out = Command::new("/usr/bin/sudo")
            .arg("-n")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "sudo {args:?}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    sample_root(&root);
    let data = root.join("Library/Russet");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("big.bin"), vec![7u8; 300_000]).unwrap();
    fs::write(data.join("private"), "secret").unwrap();
    fs::set_permissions(data.join("private"), fs::Permissions::from_mode(0o600)).unwrap();
    let mut nodes = nodes_as_root(&root);
    for node in &mut nodes {
        if node.path == "Library/Russet/private" {
            (node.uid, node.gid) = (501, 20);
        }
    }

    let identifier = format!("com.example.russet-install-test.{}", std::process::id());
    let destination = format!("/private/tmp/russet-install-test-{}", std::process::id());
    let package = temp.path().join("Test.pkg");
    build(
        &nodes,
        &Options {
            identifier: &identifier,
            version: "1.0",
            install_location: Some(&destination),
            min_os_version: None,
            scripts: None,
            info_template: None,
            components: &[],
        },
        &package,
    )
    .unwrap();

    if let Ok(dir) = std::env::var("RUSSET_KEEP_TEST_PACKAGE") {
        fs::copy(&package, Path::new(&dir).join("Test.pkg")).unwrap();
        return;
    }
    let result = std::panic::catch_unwind(|| {
        sudo(&[
            "/usr/sbin/installer",
            "-pkg",
            package.to_str().unwrap(),
            "-target",
            "/",
        ]);
        // The receipt lists every path in the payload.
        let mut receipt: Vec<String> = Command::new("/usr/sbin/pkgutil")
            .args(["--files", &identifier])
            .output()
            .unwrap()
            .stdout
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| String::from_utf8(l.to_vec()).unwrap())
            .collect();
        receipt.sort();
        let mut expected: Vec<String> = nodes
            .iter()
            .filter(|n| !n.path.is_empty())
            .map(|n| n.path.clone())
            .collect();
        expected.sort();
        assert_eq!(receipt, expected);
        // Each installed item has the recorded owner, mode, and contents.
        for node in nodes.iter().filter(|n| !n.path.is_empty()) {
            let installed = Path::new(&destination).join(&node.path);
            let metadata = fs::symlink_metadata(&installed).unwrap();
            assert_eq!(
                (metadata.uid(), metadata.gid()),
                (node.uid, node.gid),
                "{}",
                node.path
            );
            if !metadata.file_type().is_symlink() {
                assert_eq!(
                    metadata.mode() & 0o7777,
                    u32::from(node.mode),
                    "{}",
                    node.path
                );
            }
            if let NodeKind::File(source) = &node.kind {
                assert_eq!(
                    fs::read(&installed).unwrap(),
                    fs::read(source).unwrap(),
                    "{}",
                    node.path
                );
            }
        }
    });
    sudo(&["/usr/sbin/pkgutil", "--forget", &identifier]);
    sudo(&["/bin/rm", "-rf", &destination]);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
