//! Verifies a real Developer ID signed bundle, then tampered copies of it.
//! These tests need no macOS tools, so they run on Linux too. Windows
//! checks out the bundle's symlinks as plain files, and Russet doesn't
//! verify code signatures there.
#![cfg(unix)]

use russet_codesign::bundle::{verify, Options};
use russet_codesign::requirement::{Context, Requirement};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DESIGNATED: &str = r#"identifier "com.googlecode.munki.MSCDockTilePlugin" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = T4SK8ZXCXG"#;
const OPTIONS: Options = Options {
    deep: true,
    strict: true,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/MSCDockTilePlugin.docktileplugin")
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn satisfies(path: &Path, requirement: &str) -> Result<bool, String> {
    let signature = verify(path, OPTIONS, SystemTime::now())?;
    let context = Context {
        identifier: &signature.identifier,
        chain: signature.chain.as_ref(),
        cdhashes: &signature.cdhashes,
    };
    Ok(Requirement::parse(requirement)?.evaluate(&context))
}

#[test]
fn verifies_a_developer_id_bundle() {
    let signature = verify(&fixture(), OPTIONS, SystemTime::now()).unwrap();
    assert_eq!(
        signature.identifier,
        "com.googlecode.munki.MSCDockTilePlugin"
    );
    assert_eq!(signature.team.as_deref(), Some("T4SK8ZXCXG"));
    assert!(signature.timestamp.is_some());
    assert_eq!(
        signature.chain.unwrap().names(),
        [
            "Developer ID Application: Mac Admins Open Source (T4SK8ZXCXG)",
            "Developer ID Certification Authority",
            "Apple Root CA"
        ]
    );
    assert!(satisfies(&fixture(), DESIGNATED).unwrap());
    assert!(!satisfies(&fixture(), &DESIGNATED.replace("T4SK8ZXCXG", "XXXXXXXXXX")).unwrap());
    assert!(!satisfies(
        &fixture(),
        r#"identifier "com.example.other" and anchor apple generic"#
    )
    .unwrap());
}

#[test]
fn rejects_tampered_copies() {
    let temp = tempfile::tempdir().unwrap();
    type Change = fn(&Path);
    let tamper: [(&str, Change); 4] = [
        ("executable", |b| {
            let path = b.join("Contents/MacOS/MSCDockTilePlugin");
            let mut bytes = fs::read(&path).unwrap();
            bytes[20_000] ^= 1;
            fs::write(path, bytes).unwrap();
        }),
        ("Info.plist", |b| {
            let path = b.join("Contents/Info.plist");
            let text = fs::read_to_string(&path)
                .unwrap()
                .replace("<dict>", "<dict><key>X</key><true/>");
            fs::write(path, text).unwrap();
        }),
        ("added file", |b| {
            fs::write(b.join("Contents/extra.txt"), "x").unwrap()
        }),
        ("resource seal", |b| {
            let path = b.join("Contents/_CodeSignature/CodeResources");
            let text = fs::read_to_string(&path)
                .unwrap()
                .replace("<dict>", "<dict><key>X</key><true/>");
            fs::write(path, text).unwrap();
        }),
    ];
    for (name, change) in tamper {
        let bundle = temp
            .path()
            .join(name.replace(' ', "-"))
            .join("Plugin.docktileplugin");
        copy(&fixture(), &bundle);
        assert!(
            verify(&bundle, OPTIONS, SystemTime::now()).is_ok(),
            "{name}: copy should verify"
        );
        change(&bundle);
        assert!(
            verify(&bundle, OPTIONS, SystemTime::now()).is_err(),
            "{name}: tampering wasn't detected"
        );
    }
}

#[cfg(unix)]
#[test]
fn strict_mode_rejects_finder_info() {
    let temp = tempfile::tempdir().unwrap();
    let bundle = temp.path().join("Plugin.docktileplugin");
    copy(&fixture(), &bundle);
    let name = if cfg!(target_os = "linux") {
        "user.com.apple.FinderInfo"
    } else {
        "com.apple.FinderInfo"
    };
    let mut info = [0u8; 32];
    info[8] = 4;
    if xattr::set(&bundle, name, &info).is_err() {
        return; // The filesystem doesn't support extended attributes.
    }
    let error = verify(&bundle, OPTIONS, SystemTime::now()).unwrap_err();
    assert!(error.contains("detritus"), "{error}");
    assert!(verify(
        &bundle,
        Options {
            deep: true,
            strict: false
        },
        SystemTime::now()
    )
    .is_ok());
}
