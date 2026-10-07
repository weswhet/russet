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

/// Streaming matches checksumming each slice in memory, for slices that
/// cross read boundaries, and drops a slice that runs past the file.
#[test]
fn scan_streams_universal_slices() {
    let mut file = vec![0u8; 200_000];
    file[..8].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 3]);
    let slices: [(u32, u32, u32, u32); 3] = [
        (0x0100_0007, 3, 4096, 70_000),
        (0x0100_000c, 0, 90_000, 100_000),
        (7, 3, 150_000, 100_000),
    ];
    for (i, (cpu, sub, offset, size)) in slices.iter().enumerate() {
        let at = 8 + i * 20;
        for (j, value) in [cpu, sub, offset, size].into_iter().enumerate() {
            file[at + j * 4..at + j * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
    }
    for (i, byte) in file.iter_mut().enumerate().skip(100) {
        *byte = (i * 7 % 251) as u8;
    }
    let mut copy = Vec::new();
    let summary = crate::scan(file.as_slice(), &mut copy).unwrap();
    assert_eq!(copy, file);
    assert_eq!(summary.size, 200_000);
    assert_eq!(summary.checksum, cksum(&file));
    assert_eq!(summary.archs.len(), 2);
    for (arch, (cpu, _, offset, size)) in summary.archs.iter().zip(slices) {
        assert_eq!(arch.cpu_type, cpu);
        assert_eq!(
            arch.checksum,
            cksum(&file[offset as usize..(offset + size) as usize])
        );
    }
}

/// Files of 4 GiB or more keep their full size through the Size64 tree.
#[test]
fn round_trips_large_files() {
    let large = |path: &str, size: u64| {
        entry(
            path,
            Kind::File {
                size,
                checksum: 1,
                archs: Vec::new(),
            },
            0o644,
        )
    };
    let entries = vec![
        entry("", Kind::Directory, 0o755),
        large("a", 5 << 30),
        large("b", 3),
        large("c", (1 << 32) + 7),
    ];
    assert_eq!(read(&write(&entries).unwrap()).unwrap(), entries);
}

/// `lsbom` lists a native BOM with a 5 GiB file as it lists `mkbom`'s.
#[cfg(target_os = "macos")]
#[test]
fn large_files_match_apple_mkbom() {
    use std::fs;
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    // Sparse, so it takes no disk space.
    fs::File::create(root.join("big"))
        .unwrap()
        .set_len(5 << 30)
        .unwrap();
    fs::write(root.join("small"), "hi\n").unwrap();
    let apple_bom = temp.path().join("apple.bom");
    assert!(Command::new("/usr/bin/mkbom")
        .arg(&root)
        .arg(&apple_bom)
        .status()
        .unwrap()
        .success());
    let entries = read(&fs::read(&apple_bom).unwrap()).unwrap();
    assert!(entries
        .iter()
        .any(|e| matches!(e.kind, Kind::File { size, .. } if size == 5 << 30)));
    let native_bom = temp.path().join("native.bom");
    fs::write(&native_bom, write(&entries).unwrap()).unwrap();
    let listing = |path: &std::path::Path| {
        let out = Command::new("/usr/bin/lsbom")
            .args(["-p", "fMUGsc"])
            .arg(path)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    assert_eq!(listing(&native_bom), listing(&apple_bom));
    assert!(listing(&native_bom).contains("5368709120"));
}
