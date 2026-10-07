use crate::{cksum, macho_archs, read, write, Entry, Kind};

fn entry(path: &str, kind: Kind, mode: u16) -> Entry {
    Entry {
        path: path.into(),
        kind,
        mode,
        uid: 0,
        gid: 80,
        mtime: 1_700_000_000,
    }
}

fn file(path: &str, data: &[u8]) -> Entry {
    entry(
        path,
        Kind::File {
            size: data.len() as u64,
            checksum: cksum(data),
            archs: macho_archs(data),
        },
        0o644,
    )
}

#[test]
fn cksum_matches_posix() {
    // Values from `cksum` on macOS.
    assert_eq!(cksum(b"file"), 4_152_806_985);
    assert_eq!(cksum(b"hello\n"), 3_015_617_425);
    assert_eq!(cksum(b""), 4_294_967_295);
}

#[test]
fn round_trips() {
    let entries = vec![
        entry("", Kind::Directory, 0o755),
        entry("Applications", Kind::Directory, 0o775),
        entry("Applications/Tool.app", Kind::Directory, 0o755),
        file("Applications/Tool.app/Info.plist", b"<plist/>"),
        entry(
            "Applications/Tool.app/Link",
            Kind::Symlink {
                target: "Info.plist".into(),
                checksum: cksum(b"Info.plist"),
            },
            0o755,
        ),
    ];
    assert_eq!(read(&write(&entries).unwrap()).unwrap(), entries);
}

#[test]
fn round_trips_across_many_leaves() {
    let mut entries = vec![
        entry("", Kind::Directory, 0o755),
        entry("files", Kind::Directory, 0o755),
    ];
    for i in 0..1200 {
        entries.push(file(&format!("files/f{i:04}"), format!("{i}").as_bytes()));
    }
    assert_eq!(read(&write(&entries).unwrap()).unwrap(), entries);
}

#[test]
fn rejects_bad_input() {
    assert!(write(&[file("a", b"x")]).is_err());
    assert!(write(&[entry("", Kind::Directory, 0o755), file("missing/a", b"x")]).is_err());
    assert!(read(b"not a bom").is_err());
}

/// Compares with Apple's `mkbom` and `lsbom`: Apple's tool reads our BOMs,
/// and we read Apple's, with the same listing.
#[cfg(target_os = "macos")]
#[test]
fn matches_apple_mkbom_and_lsbom() {
    use std::fs;
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin/data.txt"), "hello\n").unwrap();
    fs::copy("/usr/bin/true", root.join("bin/true")).unwrap();
    std::os::unix::fs::symlink("data.txt", root.join("bin/link")).unwrap();
    let apple_bom = temp.path().join("apple.bom");
    assert!(Command::new("/usr/bin/mkbom")
        .arg(&root)
        .arg(&apple_bom)
        .status()
        .unwrap()
        .success());
    let lsbom = |path: &std::path::Path| -> Vec<String> {
        let out = Command::new("/usr/bin/lsbom").arg(path).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let apple_listing = lsbom(&apple_bom);
    let ours = read(&fs::read(&apple_bom).unwrap()).unwrap();
    assert_eq!(
        ours.iter().map(Entry::lsbom_line).collect::<Vec<_>>(),
        apple_listing
    );
    let native_bom = temp.path().join("native.bom");
    fs::write(&native_bom, write(&ours).unwrap()).unwrap();
    assert_eq!(lsbom(&native_bom), apple_listing);
    // Architecture details survive too.
    let detail = |path: &std::path::Path| -> String {
        let out = Command::new("/usr/bin/lsbom")
            .args(["-p", "fMUGsc"])
            .arg("-a")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(detail(&native_bom), detail(&apple_bom));
}
