//! Checks native extraction against manifests captured from images that
//! `hdiutil` mounted on macOS. Runs on every Unix platform.

use crate::{extract, image_info};
use russet_fs::{manifest, Limits};
use std::path::Path;

#[test]
fn fixtures_match_mounted_manifests() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let image = entry.unwrap().path();
        if image.extension().is_none_or(|e| e != "dmg") {
            continue;
        }
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(image.with_extension("json")).unwrap()).unwrap();
        assert_eq!(
            image_info(&image).unwrap().format,
            expected["format"].as_str().unwrap(),
            "{}",
            image.display()
        );
        let out = tempfile::tempdir().unwrap();
        let extraction = extract(&image, out.path(), Limits::default()).unwrap();
        let volumes: Vec<_> = extraction
            .volumes
            .iter()
            .map(|v| serde_json::to_value(manifest(v).unwrap()).unwrap())
            .collect();
        assert_eq!(
            serde_json::Value::Array(volumes),
            expected["volumes"],
            "{}",
            image.display()
        );
        checked += 1;
    }
    assert_eq!(checked, 8);
}

#[test]
fn rejects_non_images_and_limits_size() {
    let temp = tempfile::tempdir().unwrap();
    let bogus = temp.path().join("bogus.dmg");
    std::fs::write(&bogus, vec![0u8; 4096]).unwrap();
    let error = extract(&bogus, temp.path(), Limits::default()).unwrap_err();
    assert!(error.to_string().contains("isn't a disk image"), "{error}");

    let encrypted = temp.path().join("encrypted.dmg");
    std::fs::write(&encrypted, b"encrcdsa\0\0\0\0").unwrap();
    let error = extract(&encrypted, temp.path(), Limits::default()).unwrap_err();
    assert!(error.to_string().contains("encrypted"), "{error}");

    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/UDZO-HFSP.dmg");
    let out = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_total_bytes: 1000,
        ..Limits::default()
    };
    let error = extract(&image, out.path(), limits).unwrap_err();
    assert!(error.to_string().contains("limit"), "{error}");
}

/// Creates images in each supported format and reads them back.
#[test]
fn created_images_round_trip() {
    use crate::{create, CreateOptions};
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Fixture");
    std::fs::create_dir_all(source.join("App.app/Contents/MacOS")).unwrap();
    std::fs::write(source.join("App.app/Contents/MacOS/App"), "binary").unwrap();
    std::fs::set_permissions(
        source.join("App.app/Contents/MacOS/App"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(source.join("Español.txt"), "accent").unwrap();
    std::os::unix::fs::symlink("App.app", source.join("Link")).unwrap();
    std::fs::write(source.join("big.bin"), vec![7u8; 300_000]).unwrap();
    // A Linux extraction's attribute sidecar stays off the volume.
    std::fs::create_dir_all(source.join(".russet-xattrs/big.bin")).unwrap();
    std::fs::write(source.join(".russet-xattrs/big.bin/user.test"), "x").unwrap();
    let strip =
        |entries: Vec<russet_fs::Entry>| -> Vec<(String, u32, Option<String>, Option<String>)> {
            use unicode_normalization::UnicodeNormalization;
            entries
                .into_iter()
                .map(|e| (e.path.nfc().collect(), e.mode, e.sha256, e.target))
                .collect()
        };
    let expected = strip(manifest(&source).unwrap());
    for (format, filesystem) in [
        ("UDZO", "HFS+"),
        ("UDBZ", "Journaled HFS+"),
        ("ULFO", "APFS"),
        ("UDRO", "Case-insensitive APFS"),
    ] {
        let image = temp.path().join(format!("{format}.dmg"));
        let written = create(
            &source,
            &image,
            &CreateOptions {
                filesystem,
                format,
                zlib_level: 5,
                megabytes: None,
            },
        )
        .unwrap();
        assert_eq!(written, "HFS+");
        assert_eq!(image_info(&image).unwrap().format, format);
        let out = tempfile::tempdir().unwrap();
        let extraction = extract(&image, out.path(), Limits::default()).unwrap();
        assert_eq!(
            strip(manifest(&extraction.volumes[0]).unwrap()),
            expected,
            "{format}"
        );
    }
    // An app given as the source folder goes on the volume as itself, the way
    // `hdiutil` treats packages, so a Munki import finds it.
    let app = source.join("App.app");
    let image = temp.path().join("app.dmg");
    create(
        &app,
        &image,
        &CreateOptions {
            filesystem: "HFS+",
            format: "UDZO",
            zlib_level: 5,
            megabytes: None,
        },
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let extraction = extract(&image, out.path(), Limits::default()).unwrap();
    let mut expected: Vec<_> = strip(manifest(&app).unwrap())
        .into_iter()
        .map(|(path, mode, sha, target)| (format!("App.app/{path}"), mode, sha, target))
        .collect();
    let top = manifest(&source)
        .unwrap()
        .into_iter()
        .find(|e| e.path == "App.app")
        .unwrap();
    expected.push(("App.app".into(), top.mode, None, None));
    expected.sort();
    let mut got = strip(manifest(&extraction.volumes[0]).unwrap());
    got.sort();
    assert_eq!(got, expected);
    let error = create(
        &source,
        &temp.path().join("x.dmg"),
        &CreateOptions {
            filesystem: "Case-sensitive APFS",
            format: "UDZO",
            zlib_level: 5,
            megabytes: None,
        },
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("isn't supported natively"),
        "{error}"
    );
}

/// Images from tools other than `hdiutil` can store runs far larger than
/// the 1 MiB `hdiutil` writes, and installer volumes often declare much more
/// space than they use. Both must extract, within a limit that counts only
/// the data.
#[test]
fn large_runs_and_mostly_empty_volumes_extract() {
    use hfsplus::testutil::HfsPlusImageBuilder;
    let big: Vec<u8> = (0..65u32 << 20).map(|i| (i % 251) as u8).collect();
    let mut volume = HfsPlusImageBuilder::new()
        .add_file("big.bin", &big, 0o644)
        .build();
    let used = volume.len();
    volume.resize(used + (256 << 20), 0);
    let temp = tempfile::tempdir().unwrap();
    for method in [udif::CompressionMethod::Raw, udif::CompressionMethod::Zlib] {
        let image = temp.path().join(format!("{method:?}.dmg"));
        let mut writer = udif::create(&image)
            .unwrap()
            .compression(method)
            .compression_level(1)
            .chunk_size(used);
        writer.add_partition("disk image", &volume).unwrap();
        writer.finish().unwrap();
        let out = tempfile::tempdir().unwrap();
        let limits = Limits {
            max_total_bytes: 200 << 20,
            ..Limits::default()
        };
        let extraction = extract(&image, out.path(), limits).unwrap();
        let extracted = std::fs::read(extraction.volumes[0].join("big.bin")).unwrap();
        assert!(extracted == big, "{method:?}");
    }
}

/// Inputs the `hdiutil_extract` fuzz target found; each must fail cleanly
/// instead of allocating what a crafted header claims.
#[test]
fn fuzz_regression_inputs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fuzz-regressions");
    for entry in std::fs::read_dir(dir).unwrap() {
        let input = entry.unwrap().path();
        let out = tempfile::tempdir().unwrap();
        let _ = image_info(&input);
        assert!(
            extract(&input, out.path(), Limits::default()).is_err(),
            "{}",
            input.display()
        );
    }
}
