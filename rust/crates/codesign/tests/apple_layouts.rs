//! Signature layouts found in real downloads, signed ad hoc with `codesign`
//! and compared with it, untouched and tampered:
//! - non-Mach-O files in nested-code locations (such as `Contents/MacOS`),
//!   which `codesign` signs as code and stores in `com.apple.cs.*`
//!   extended attributes (VLC);
//! - a framework without an executable, whose signature is stored as
//!   separate files in `_CodeSignature` (Adium).
#![cfg(target_os = "macos")]

use russet_codesign::bundle::{verify, Options};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

const OPTIONS: Options = Options {
    deep: true,
    strict: true,
};

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn apple_accepts(path: &Path) -> bool {
    Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(path)
        .output()
        .unwrap()
        .status
        .success()
}

fn plist(path: &Path, identifier: &str, executable: Option<&str>) {
    let executable = executable
        .map(|e| format!("<key>CFBundleExecutable</key><string>{e}</string>"))
        .unwrap_or_default();
    fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{identifier}</string><key>CFBundlePackageType</key><string>APPL</string>{executable}</dict></plist>"#
        ),
    )
    .unwrap();
}

/// An app with a header in Contents/MacOS and a versioned framework with no
/// executable, signed inside out.
fn build(root: &Path) -> std::path::PathBuf {
    let app = root.join("Layouts.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS/include")).unwrap();
    fs::copy("/usr/bin/true", contents.join("MacOS/Layouts")).unwrap();
    plist(
        &contents.join("Info.plist"),
        "com.example.layouts",
        Some("Layouts"),
    );
    fs::write(contents.join("MacOS/include/header.h"), "#define X 1\n").unwrap();
    let framework = contents.join("Frameworks/Data.framework");
    fs::create_dir_all(framework.join("Versions/A/Resources")).unwrap();
    plist(
        &framework.join("Versions/A/Resources/Info.plist"),
        "com.example.data",
        Some("Data.framework"),
    );
    fs::write(framework.join("Versions/A/Resources/data.txt"), "data\n").unwrap();
    std::os::unix::fs::symlink("A", framework.join("Versions/Current")).unwrap();
    std::os::unix::fs::symlink("Versions/Current/Resources", framework.join("Resources")).unwrap();
    for target in [
        framework.as_path(),
        &contents.join("MacOS/include/header.h"),
        &app,
    ] {
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(target));
    }
    app
}

#[test]
fn matches_codesign_on_attribute_and_detached_signatures() {
    let temp = tempfile::tempdir().unwrap();
    let app = build(temp.path());
    let header = app.join("Contents/MacOS/include/header.h");
    assert!(
        xattr::get(&header, "com.apple.cs.CodeDirectory")
            .unwrap()
            .is_some(),
        "codesign didn't sign the header with attributes"
    );
    let framework = app.join("Contents/Frameworks/Data.framework");
    assert!(framework
        .join("Versions/A/_CodeSignature/CodeDirectory")
        .is_file());
    assert!(apple_accepts(&app));
    verify(&app, OPTIONS, SystemTime::now()).unwrap();

    type Tamper = fn(&Path);
    let cases: [(&str, Tamper); 4] = [
        ("header contents", |app| {
            fs::write(app.join("Contents/MacOS/include/header.h"), "#define X 2\n").unwrap()
        }),
        ("header signature", |app| {
            xattr::remove(
                app.join("Contents/MacOS/include/header.h"),
                "com.apple.cs.CodeDirectory",
            )
            .unwrap()
        }),
        ("framework Info.plist", |app| {
            plist(
                &app.join("Contents/Frameworks/Data.framework/Versions/A/Resources/Info.plist"),
                "com.example.other",
                Some("Data.framework"),
            )
        }),
        ("framework resource", |app| {
            fs::write(
                app.join("Contents/Frameworks/Data.framework/Versions/A/Resources/data.txt"),
                "changed\n",
            )
            .unwrap()
        }),
    ];
    for (name, tamper) in cases {
        let copy = tempfile::tempdir().unwrap();
        let app = build(copy.path());
        tamper(&app);
        assert!(!apple_accepts(&app), "codesign accepted tampered {name}");
        assert!(
            verify(&app, OPTIONS, SystemTime::now()).is_err(),
            "native verifier accepted tampered {name}"
        );
    }
}

/// `codesign --strict` rejects Finder info or a resource fork on the bundle
/// or on a file in it, but not on a folder inside it (OmniOutliner's
/// document templates).
#[test]
fn detritus_matches_codesign() {
    let finder_info = {
        let mut info = vec![0u8; 32];
        info[8] = 0x40;
        info
    };
    let cases: [(&str, &str, &str); 5] = [
        (
            "folder",
            "Contents/Resources/Doc.pkgdir",
            "com.apple.FinderInfo",
        ),
        ("Contents", "Contents", "com.apple.FinderInfo"),
        (
            "file",
            "Contents/Resources/file.txt",
            "com.apple.FinderInfo",
        ),
        ("bundle", "", "com.apple.FinderInfo"),
        (
            "resource fork",
            "Contents/Resources/file.txt",
            "com.apple.ResourceFork",
        ),
    ];
    for (name, path, attribute) in cases {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Detritus.app");
        let contents = app.join("Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        fs::create_dir_all(contents.join("Resources/Doc.pkgdir")).unwrap();
        fs::copy("/usr/bin/true", contents.join("MacOS/Detritus")).unwrap();
        plist(
            &contents.join("Info.plist"),
            "com.example.detritus",
            Some("Detritus"),
        );
        fs::write(contents.join("Resources/file.txt"), "a\n").unwrap();
        fs::write(contents.join("Resources/Doc.pkgdir/inner"), "b\n").unwrap();
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(&app));
        let value: &[u8] = if attribute == "com.apple.FinderInfo" {
            &finder_info
        } else {
            b"fork"
        };
        xattr::set(app.join(path), attribute, value).unwrap();
        let apple = apple_accepts(&app);
        let native = verify(&app, OPTIONS, SystemTime::now()).is_ok();
        assert_eq!(native, apple, "{name}");
    }
}

/// A bundle whose executable is a validly signed standalone tool, here
/// Apple's own `/usr/bin/true`, whose signature seals no resources or
/// Info.plist. Whatever else the bundle ships is unchecked, so `codesign`
/// rejects it, and so must the native verifier, even though the tool's own
/// signature would satisfy `anchor apple`.
#[test]
fn rejects_bundle_whose_executable_seals_no_resources() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("Trojan.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::create_dir_all(contents.join("Resources")).unwrap();
    fs::create_dir_all(contents.join("_CodeSignature")).unwrap();
    fs::copy("/usr/bin/true", contents.join("MacOS/Trojan")).unwrap();
    plist(
        &contents.join("Info.plist"),
        "com.apple.true",
        Some("Trojan"),
    );
    fs::write(contents.join("Resources/payload.sh"), "#!/bin/sh\n").unwrap();
    fs::write(
        contents.join("_CodeSignature/CodeResources"),
        r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>files</key><dict/><key>files2</key><dict/><key>rules</key><dict/><key>rules2</key><dict/></dict></plist>"#,
    )
    .unwrap();
    assert!(!apple_accepts(&app));
    let error = verify(&app, OPTIONS, SystemTime::now()).unwrap_err();
    assert!(error.contains("no resources"), "{error}");
}

/// A flat framework keeps its Info.plist in Resources, as Firefox's
/// ChannelPrefs.framework does, and codesign accepts it.
#[test]
fn accepts_flat_framework_with_info_plist_in_resources() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("Flat.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::copy("/usr/bin/true", contents.join("MacOS/Flat")).unwrap();
    plist(
        &contents.join("Info.plist"),
        "com.example.flat",
        Some("Flat"),
    );
    let framework = contents.join("Frameworks/Flat.framework");
    fs::create_dir_all(framework.join("Resources")).unwrap();
    fs::copy("/usr/bin/true", framework.join("Flat")).unwrap();
    plist(
        &framework.join("Resources/Info.plist"),
        "com.example.flatframework",
        Some("Flat"),
    );
    for target in [framework.as_path(), &app] {
        run(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(target));
    }
    assert!(apple_accepts(&app));
    verify(&app, OPTIONS, SystemTime::now()).unwrap();
}

/// An app whose main executable is a shell script. `codesign` signs it with
/// separate files in `_CodeSignature`, as in OpenShot.
fn script_app(root: &Path) -> std::path::PathBuf {
    let app = root.join("Script.app");
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::write(contents.join("MacOS/Script"), "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(
        contents.join("MacOS/Script"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    plist(
        &contents.join("Info.plist"),
        "com.example.script",
        Some("Script"),
    );
    run(Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app));
    app
}

#[test]
fn accepts_script_main_executable_with_detached_signature() {
    let temp = tempfile::tempdir().unwrap();
    let app = script_app(temp.path());
    assert!(app.join("Contents/_CodeSignature/CodeDirectory").is_file());
    assert!(apple_accepts(&app));
    verify(&app, OPTIONS, SystemTime::now()).unwrap();
}

#[test]
fn rejects_script_main_executable_with_appended_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let app = script_app(temp.path());
    fs::OpenOptions::new()
        .append(true)
        .open(app.join("Contents/MacOS/Script"))
        .unwrap()
        .write_all(b"echo injected\n")
        .unwrap();
    assert!(!apple_accepts(&app));
    assert!(verify(&app, OPTIONS, SystemTime::now()).is_err());
}
