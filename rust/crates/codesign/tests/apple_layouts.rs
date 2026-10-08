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
