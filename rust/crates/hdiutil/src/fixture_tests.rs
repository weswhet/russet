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
    assert!(error.to_string().contains("size limit"), "{error}");
}
