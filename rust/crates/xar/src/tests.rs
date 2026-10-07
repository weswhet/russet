use crate::{Archive, Builder, Content, Encoding, EntryKind};
use russet_fs::{manifest, Limits};
use std::fs;
use std::path::Path;

fn sample(path: &Path) {
    let mut builder = Builder::new();
    builder
        .add_file(
            Path::new("Distribution"),
            0o644,
            Content::Bytes(b"<installer-gui-script/>".to_vec()),
            Encoding::Bzip2,
        )
        .unwrap();
    builder.add_directory(Path::new("Tool.pkg"), 0o700).unwrap();
    builder
        .add_file(
            Path::new("Tool.pkg/PackageInfo"),
            0o644,
            Content::Bytes(b"<pkg-info/>".to_vec()),
            Encoding::Zlib,
        )
        .unwrap();
    builder
        .add_file(
            Path::new("Tool.pkg/Payload"),
            0o644,
            Content::Bytes(vec![7; 5000]),
            Encoding::None,
        )
        .unwrap();
    builder.write(path).unwrap();
}

#[test]
fn round_trip_with_modes_and_checksums() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.pkg");
    sample(&path);
    let mut archive = Archive::open(&path).unwrap();
    assert!(archive.checksum().is_some());
    let kinds: Vec<_> = archive
        .entries()
        .iter()
        .map(|e| (e.path.to_string_lossy().into_owned(), e.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Distribution".into(), EntryKind::File),
            ("Tool.pkg".into(), EntryKind::Directory),
            ("Tool.pkg/PackageInfo".into(), EntryKind::File),
            ("Tool.pkg/Payload".into(), EntryKind::File),
        ]
    );
    assert_eq!(
        archive.read("Tool.pkg/PackageInfo", 1 << 20).unwrap(),
        b"<pkg-info/>"
    );
    let out = temp.path().join("out");
    fs::create_dir(&out).unwrap();
    archive.extract(&out, Limits::default(), |_| false).unwrap();
    let entries = manifest(&out).unwrap();
    assert_eq!(
        entries.iter().find(|e| e.path == "Tool.pkg").unwrap().mode,
        0o700
    );
    assert_eq!(
        fs::read(out.join("Tool.pkg/Payload")).unwrap(),
        vec![7; 5000]
    );
}

/// Locates `needle` in the file, so tests can corrupt precise bytes.
fn find(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap()
}

#[test]
fn rejects_tampering() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.pkg");
    sample(&path);
    let original = fs::read(&path).unwrap();

    // Changed file data fails its archived checksum when read.
    let mut bytes = original.clone();
    let at = find(&bytes, &[7; 64]);
    bytes[at] = 8;
    fs::write(&path, &bytes).unwrap();
    let mut archive = Archive::open(&path).unwrap();
    let error = archive.read("Tool.pkg/Payload", 1 << 20).unwrap_err();
    assert!(error.to_string().contains("checksum"), "{error}");

    // A changed TOC checksum is rejected when the archive opens.
    let mut bytes = original.clone();
    let heap = 28 + u64::from_be_bytes(bytes[8..16].try_into().unwrap()) as usize;
    bytes[heap] ^= 1;
    fs::write(&path, &bytes).unwrap();
    let error = Archive::open(&path).err().unwrap();
    assert!(
        error.to_string().contains("TOC checksum doesn't match"),
        "{error}"
    );

    // A truncated file is rejected.
    fs::write(&path, &original[..original.len() - 100]).unwrap();
    assert!(Archive::open(&path).is_err());

    fs::write(&path, b"not a xar archive at all, just some bytes").unwrap();
    assert!(Archive::open(&path)
        .err()
        .unwrap()
        .to_string()
        .contains("isn't a xar archive"));
}

/// Rebuilds an archive around an edited TOC, with a correct checksum, so
/// structural checks are tested rather than the checksum.
fn with_toc(path: &Path, edit: impl Fn(&str) -> String) {
    let bytes = fs::read(path).unwrap();
    let compressed_len = u64::from_be_bytes(bytes[8..16].try_into().unwrap()) as usize;
    let mut xml = String::new();
    use std::io::Read;
    flate2::read::ZlibDecoder::new(&bytes[28..28 + compressed_len])
        .read_to_string(&mut xml)
        .unwrap();
    let xml = edit(&xml);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, xml.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut out = bytes[..8].to_vec();
    out.extend_from_slice(&(compressed.len() as u64).to_be_bytes());
    out.extend_from_slice(&(xml.len() as u64).to_be_bytes());
    out.extend_from_slice(&bytes[24..28]);
    out.extend_from_slice(&compressed);
    out.extend_from_slice(&crate::Algorithm::Sha1.digest(&compressed));
    out.extend_from_slice(&bytes[28 + compressed_len + 20..]);
    fs::write(path, out).unwrap();
}

#[test]
fn rejects_structural_attacks() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.pkg");
    type Edit = fn(&str) -> String;
    let cases: [(&str, Edit); 4] = [
        ("exactly one checksum", |x| {
            x.replacen("<checksum style=\"sha1\">", "<checksum style=\"sha1\"><size>20</size><offset>0</offset></checksum>\n  <checksum style=\"sha1\">", 1)
        }),
        ("invalid name", |x| {
            x.replacen("<name>Distribution</name>", "<name>../evil</name>", 1)
        }),
        ("overlaps", |x| {
            x.replacen("<offset>20</offset>", "<offset>0</offset>", 1)
        }),
        ("out of range", |x| {
            x.replacen("<length>5000</length>", "<length>999999</length>", 1)
        }),
    ];
    for (expected, edit) in cases {
        sample(&path);
        with_toc(&path, edit);
        let error = Archive::open(&path)
            .err()
            .unwrap_or_else(|| panic!("{expected}"));
        assert!(error.to_string().contains(expected), "{expected}: {error}");
    }
}

/// Compares extraction with Apple's `xar` on packages `pkgbuild` and
/// `productbuild` made.
#[cfg(target_os = "macos")]
#[test]
fn matches_apple_xar() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root/Applications/Tool.app/Contents");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Info.plist"), "<plist/>").unwrap();
    let scripts = temp.path().join("scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::write(scripts.join("postinstall"), "#!/bin/sh\nexit 0\n").unwrap();
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
    for name in ["component.pkg", "product.pkg"] {
        let apple = temp.path().join(format!("apple-{name}"));
        fs::create_dir(&apple).unwrap();
        run(&[
            "/usr/bin/xar",
            "-xf",
            &t(name),
            "-C",
            apple.to_str().unwrap(),
        ]);
        let native = temp.path().join(format!("native-{name}"));
        fs::create_dir(&native).unwrap();
        Archive::open(&temp.path().join(name))
            .unwrap()
            .extract(&native, Limits::default(), |_| false)
            .unwrap();
        let strip = |m: Vec<russet_fs::Entry>| -> Vec<_> {
            m.into_iter()
                .map(|mut e| {
                    e.xattrs.clear();
                    e
                })
                .collect()
        };
        assert_eq!(
            strip(manifest(&native).unwrap()),
            strip(manifest(&apple).unwrap()),
            "{name}"
        );
    }
}
