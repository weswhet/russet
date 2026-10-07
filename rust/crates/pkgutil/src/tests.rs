use crate::{expand, flatten};
use russet_fs::{manifest, Limits};
use std::fs;

/// Expands packages from `pkgbuild` and `productbuild` with both
/// implementations, then flattens with each and expands again with Apple's
/// tool, comparing the trees each time.
#[cfg(target_os = "macos")]
#[test]
fn matches_apple_pkgutil() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root/Applications/Tool.app/Contents");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Info.plist"), "<plist/>").unwrap();
    let scripts = temp.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::write(scripts.join("postinstall"), "#!/bin/sh\nexit 0\n").unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(
        &mut fs::metadata(scripts.join("postinstall"))
            .unwrap()
            .permissions(),
        0o755,
    );
    let run = |args: &[&str]| {
        assert!(
            Command::new(args[0])
                .args(&args[1..])
                .status()
                .unwrap()
                .success(),
            "{args:?}"
        )
    };
    let t = |p: &str| temp.path().join(p).to_string_lossy().into_owned();
    run(&["/bin/chmod", "755", &t("scripts/postinstall")]);
    run(&[
        "/usr/bin/pkgbuild",
        "--quiet",
        "--root",
        &t("root"),
        "--identifier",
        "com.example.tool",
        "--version",
        "1.0",
        "--scripts",
        &t("scripts"),
        &t("component.pkg"),
    ]);
    run(&[
        "/usr/bin/productbuild",
        "--quiet",
        "--package",
        &t("component.pkg"),
        &t("product.pkg"),
    ]);
    // Provenance and other host attributes aren't package content.
    let tree = |p: &std::path::Path| -> Vec<_> {
        manifest(p)
            .unwrap()
            .into_iter()
            .map(|mut e| {
                e.xattrs.clear();
                e
            })
            .collect()
    };
    for name in ["component.pkg", "product.pkg"] {
        run(&[
            "/usr/sbin/pkgutil",
            "--expand",
            &t(name),
            &t(&format!("apple-{name}")),
        ]);
        expand(
            &temp.path().join(name),
            &temp.path().join(format!("native-{name}")),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(
            tree(&temp.path().join(format!("native-{name}"))),
            tree(&temp.path().join(format!("apple-{name}"))),
            "expand {name}"
        );

        flatten(
            &temp.path().join(format!("apple-{name}")),
            &temp.path().join(format!("flat-{name}")),
        )
        .unwrap();
        run(&[
            "/usr/sbin/pkgutil",
            "--expand",
            &t(&format!("flat-{name}")),
            &t(&format!("again-{name}")),
        ]);
        assert_eq!(
            tree(&temp.path().join(format!("again-{name}"))),
            tree(&temp.path().join(format!("apple-{name}"))),
            "flatten {name}"
        );
    }
    let error = expand(
        &temp.path().join("product.pkg"),
        &temp.path().join("apple-product.pkg"),
        Limits::default(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("File exists") || error.to_string().contains("exists"),
        "{error}"
    );
}

#[test]
fn expand_requires_a_new_destination_and_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("expanded");
    fs::create_dir_all(folder.join("Tool.pkg/Scripts")).unwrap();
    fs::write(folder.join("Distribution"), "<installer-gui-script/>").unwrap();
    fs::write(folder.join("Tool.pkg/PackageInfo"), "<pkg-info/>").unwrap();
    fs::write(folder.join("Tool.pkg/Payload"), vec![1; 100]).unwrap();
    fs::write(folder.join("Tool.pkg/Scripts/postinstall"), "#!/bin/sh\n").unwrap();
    let package = temp.path().join("Tool.pkg");
    flatten(&folder, &package).unwrap();
    let out = temp.path().join("out");
    expand(&package, &out, Limits::default()).unwrap();
    let strip = |p: &std::path::Path| -> Vec<_> {
        manifest(p)
            .unwrap()
            .into_iter()
            .map(|mut e| {
                e.xattrs.clear();
                e
            })
            .collect()
    };
    assert_eq!(strip(&out), strip(&folder));
    assert!(expand(&package, &out, Limits::default()).is_err());
    assert!(expand(
        &package,
        &temp.path().join("missing/out"),
        Limits::default()
    )
    .is_err());
}

fn signed_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/Nudge_LaunchAgent-1.0.1.pkg")
}

#[test]
fn checks_a_developer_id_signature() {
    use std::time::{Duration, UNIX_EPOCH};
    // Signed 2026-09-08 with a certificate that expires 2028-02-09. The
    // trusted timestamp keeps it valid after the certificate expires.
    for now in [
        UNIX_EPOCH + Duration::from_secs(1_790_000_000),
        UNIX_EPOCH + Duration::from_secs(1_900_000_000),
    ] {
        let signature = crate::check_signature(&signed_fixture(), now).unwrap();
        assert_eq!(
            signature.chain.names(),
            [
                "Developer ID Installer: Mac Admins Open Source (T4SK8ZXCXG)",
                "Developer ID Certification Authority",
                "Apple Root CA"
            ]
        );
        assert_eq!(
            signature.status,
            "signed by a developer certificate issued by Apple for distribution"
        );
        assert_eq!(
            russet_codesign::package::format_time(signature.timestamp.unwrap()),
            "2026-09-08 16:21:28 +0000"
        );
    }
}

#[test]
fn rejects_unsigned_and_tampered_packages() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("expanded");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("PackageInfo"), "<pkg-info/>").unwrap();
    let unsigned = temp.path().join("unsigned.pkg");
    flatten(&folder, &unsigned).unwrap();
    let error = crate::check_signature(&unsigned, std::time::SystemTime::now()).unwrap_err();
    assert!(error.contains("no signature"), "{error}");

    // Flip one byte of the RSA signature, which sits just after the
    // 20-byte TOC checksum at the start of the heap.
    let mut bytes = fs::read(signed_fixture()).unwrap();
    let heap = 28 + u64::from_be_bytes(bytes[8..16].try_into().unwrap()) as usize;
    bytes[heap + 20 + 100] ^= 1;
    let tampered = temp.path().join("tampered.pkg");
    fs::write(&tampered, &bytes).unwrap();
    let error = crate::check_signature(&tampered, std::time::SystemTime::now()).unwrap_err();
    assert!(error.contains("signature is invalid"), "{error}");
}

/// Compares the chain with `pkgutil --check-signature`.
#[cfg(target_os = "macos")]
#[test]
fn chain_matches_apple_pkgutil() {
    let output = std::process::Command::new("/usr/sbin/pkgutil")
        .arg("--check-signature")
        .arg(signed_fixture())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    let apple: Vec<String> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let (n, rest) = l.split_once(". ")?;
            n.parse::<u32>().ok().map(|_| rest.to_owned())
        })
        .collect();
    let native = crate::check_signature(&signed_fixture(), std::time::SystemTime::now()).unwrap();
    assert_eq!(native.chain.names(), apple);
}
