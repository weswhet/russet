use crate::appledouble::tests::build as apple_double;
use crate::{extract_cpio, extract_zip};
use russet_fs::{manifest, EntryKind, Limits};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use zip::write::SimpleFileOptions;

fn zip_with(path: &Path, build: impl FnOnce(&mut zip::ZipWriter<fs::File>)) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
    build(&mut writer);
    writer.finish().unwrap();
}

fn options(mode: u32) -> SimpleFileOptions {
    SimpleFileOptions::default().unix_permissions(mode)
}

#[test]
fn zip_keeps_modes_symlinks_and_apple_double_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("app.zip");
    zip_with(&archive, |z| {
        z.add_directory("App.app/", options(0o755)).unwrap();
        z.add_directory("App.app/Contents/Frameworks/", options(0o700))
            .unwrap();
        z.start_file("App.app/Contents/MacOS/App", options(0o755))
            .unwrap();
        z.write_all(b"binary").unwrap();
        z.start_file("App.app/Contents/Info.plist", options(0o640))
            .unwrap();
        z.write_all(b"plist").unwrap();
        z.add_symlink("App.app/Contents/Current", "MacOS", options(0o755))
            .unwrap();
        // In-place metadata, as `ditto -c -k` writes it.
        z.start_file("App.app/Contents/._Info.plist", options(0o644))
            .unwrap();
        z.write_all(&apple_double(
            [0; 32],
            &[("com.example.test", b"hello")],
            b"RSRC",
        ))
        .unwrap();
        // Sequestered metadata, as `ditto -c -k --sequesterRsrc` writes it.
        z.start_file("__MACOSX/App.app/Contents/MacOS/._App", options(0o644))
            .unwrap();
        z.write_all(&apple_double([0; 32], &[("com.example.bin", b"x")], b""))
            .unwrap();
        // A `._` member that isn't AppleDouble stays an ordinary file.
        z.start_file("App.app/._notes", options(0o644)).unwrap();
        z.write_all(b"plain text").unwrap();
    });
    let out = temp.path().join("out");
    let report = extract_zip(&archive, &out, Limits::default()).unwrap();
    assert!(
        report.skipped_xattrs.is_empty(),
        "{:?}",
        report.skipped_xattrs
    );

    let entries = manifest(&out).unwrap();
    let find = |p: &str| {
        entries
            .iter()
            .find(|e| e.path == p)
            .unwrap_or_else(|| panic!("{p}"))
    };
    assert_eq!(find("App.app/Contents/MacOS/App").mode, 0o755);
    assert_eq!(find("App.app/Contents/Info.plist").mode, 0o640);
    assert_eq!(find("App.app/Contents/Frameworks").mode, 0o700);
    let link = find("App.app/Contents/Current");
    assert_eq!(
        (link.kind, link.target.as_deref()),
        (EntryKind::Symlink, Some("MacOS"))
    );
    let info = find("App.app/Contents/Info.plist");
    assert_eq!(
        info.xattrs.keys().collect::<Vec<_>>(),
        ["com.apple.ResourceFork", "com.example.test"]
    );
    assert!(find("App.app/Contents/MacOS/App")
        .xattrs
        .contains_key("com.example.bin"));
    assert_eq!(find("App.app/._notes").size, Some(10));
    assert!(entries.iter().all(|e| !e.path.contains("__MACOSX")));
    assert!(entries.iter().all(|e| !e.path.ends_with("._Info.plist")));
}

#[test]
fn zip_rejects_escapes_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("evil.zip");
    zip_with(&archive, |z| {
        z.start_file("good", options(0o644)).unwrap();
        z.start_file("../escaped", options(0o644)).unwrap();
    });
    let out = temp.path().join("out");
    let error = extract_zip(&archive, &out, Limits::default()).unwrap_err();
    assert!(error.to_string().contains("outside destination"), "{error}");
    assert!(!out.join("good").exists());
    assert!(!temp.path().join("escaped").exists());
}

#[test]
fn zip_symlink_cannot_redirect_a_later_write() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let archive = temp.path().join("evil.zip");
    zip_with(&archive, |z| {
        z.add_symlink("link", outside.to_str().unwrap(), options(0o755))
            .unwrap();
        z.start_file("link/planted", options(0o644)).unwrap();
        z.write_all(b"x").unwrap();
    });
    let out = temp.path().join("out");
    assert!(extract_zip(&archive, &out, Limits::default()).is_err());
    assert!(!outside.join("planted").exists());

    // A symlink already in the destination isn't followed either.
    let out = temp.path().join("existing");
    fs::create_dir(&out).unwrap();
    std::os::unix::fs::symlink(&outside, out.join("dir")).unwrap();
    zip_with(&archive, |z| {
        z.start_file("dir/planted", options(0o644)).unwrap();
        z.write_all(b"x").unwrap();
    });
    assert!(extract_zip(&archive, &out, Limits::default()).is_err());
    assert!(!outside.join("planted").exists());
}

#[test]
fn zip_size_limit_stops_extraction() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("big.zip");
    zip_with(&archive, |z| {
        z.start_file("big", options(0o644)).unwrap();
        z.write_all(&vec![0; 4096]).unwrap();
    });
    let limits = Limits {
        max_total_bytes: 1024,
        ..Limits::default()
    };
    let error = extract_zip(&archive, &temp.path().join("out"), limits).unwrap_err();
    assert!(error.to_string().contains("size limit"), "{error}");
}

/// Builds an odc cpio member.
fn odc(out: &mut Vec<u8>, name: &str, mode: u32, ino: u64, nlink: u64, data: &[u8]) {
    write!(
        out,
        "070707{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:06o}{:011o}{:06o}{:011o}",
        1,
        ino,
        mode,
        0,
        80,
        nlink,
        0,
        0,
        name.len() + 1,
        data.len()
    )
    .unwrap();
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(data);
}

#[test]
fn cpio_odc_and_gzip_extract_with_links() {
    let mut archive = Vec::new();
    odc(&mut archive, ".", 0o040755, 1, 2, b"");
    odc(&mut archive, "./bin", 0o040750, 2, 2, b"");
    odc(&mut archive, "./bin/tool", 0o100755, 3, 2, b"tool");
    odc(&mut archive, "./bin/alias", 0o100755, 3, 2, b"");
    odc(&mut archive, "./bin/link", 0o120755, 4, 1, b"tool");
    odc(&mut archive, "TRAILER!!!", 0, 0, 1, b"");

    let temp = tempfile::tempdir().unwrap();
    let plain = temp.path().join("payload.cpio");
    fs::write(&plain, &archive).unwrap();
    let gz = temp.path().join("payload.cpgz");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&archive).unwrap();
    fs::write(&gz, encoder.finish().unwrap()).unwrap();

    for source in [&plain, &gz] {
        let out = temp.path().join(format!(
            "out-{}",
            source.extension().unwrap().to_string_lossy()
        ));
        extract_cpio(source, &out, Limits::default()).unwrap();
        let entries = manifest(&out).unwrap();
        let find = |p: &str| entries.iter().find(|e| e.path == p).unwrap().clone();
        assert_eq!(find("bin").mode, 0o750);
        assert_eq!(find("bin/tool").mode, 0o755);
        assert_eq!(find("bin/tool").link_group, find("bin/alias").link_group);
        assert!(find("bin/tool").link_group.is_some());
        assert_eq!(find("bin/link").target.as_deref(), Some("tool"));
        assert_eq!(
            fs::metadata(out.join("bin")).unwrap().permissions().mode() & 0o777,
            0o750
        );
    }
}

#[test]
fn cpio_rejects_truncation_and_escapes() {
    let temp = tempfile::tempdir().unwrap();
    let mut archive = Vec::new();
    odc(&mut archive, "file", 0o100644, 1, 1, b"0123456789");
    archive.truncate(archive.len() - 3);
    let path = temp.path().join("short.cpio");
    fs::write(&path, &archive).unwrap();
    assert!(extract_cpio(&path, &temp.path().join("a"), Limits::default()).is_err());

    let mut archive = Vec::new();
    odc(&mut archive, "../escape", 0o100644, 1, 1, b"x");
    fs::write(&path, &archive).unwrap();
    assert!(extract_cpio(&path, &temp.path().join("b"), Limits::default()).is_err());
    assert!(!temp.path().join("escape").exists());
}

/// Compares the native extractors with Apple's `ditto` on the same archives.
#[cfg(target_os = "macos")]
mod apple {
    use super::*;
    use std::process::Command;

    fn run(args: &[&std::ffi::OsStr]) {
        let status = Command::new(args[0]).args(&args[1..]).status().unwrap();
        assert!(status.success(), "{args:?}");
    }

    fn source_tree(root: &Path) {
        let app = root.join("App.app/Contents");
        fs::create_dir_all(app.join("MacOS")).unwrap();
        fs::create_dir_all(app.join("Frameworks/F.framework/Versions/A")).unwrap();
        fs::write(app.join("MacOS/App"), "bin").unwrap();
        fs::set_permissions(app.join("MacOS/App"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(app.join("Info.plist"), "plist").unwrap();
        fs::set_permissions(app.join("Info.plist"), fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(app.join("Frameworks/F.framework/Versions/A/F"), "lib").unwrap();
        std::os::unix::fs::symlink("A", app.join("Frameworks/F.framework/Versions/Current"))
            .unwrap();
        std::os::unix::fs::symlink("Versions/Current/F", app.join("Frameworks/F.framework/F"))
            .unwrap();
        fs::hard_link(app.join("MacOS/App"), app.join("MacOS/AppLink")).unwrap();
        xattr::set(app.join("Info.plist"), "com.example.test", b"hello").unwrap();
        xattr::set(app.join("Info.plist"), "com.apple.ResourceFork", b"RSRC").unwrap();
        fs::set_permissions(app.join("Frameworks"), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn native_matches_ditto() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        source_tree(&src);
        let app = src.join("App.app");
        let ditto = std::ffi::OsStr::new("/usr/bin/ditto");
        let archives = [
            (
                "seq.zip",
                vec!["-c", "-k", "--sequesterRsrc", "--keepParent"],
                true,
            ),
            ("inplace.zip", vec!["-c", "-k", "--keepParent"], true),
            ("payload.cpgz", vec!["-c", "-z", "--keepParent"], false),
        ];
        for (name, flags, is_zip) in archives {
            let archive = temp.path().join(name);
            let mut args = vec![ditto];
            args.extend(flags.iter().map(std::ffi::OsStr::new));
            args.extend([app.as_os_str(), archive.as_os_str()]);
            run(&args);

            let apple_out = temp.path().join(format!("apple-{name}"));
            run(&[ditto, "--noqtn".as_ref(), "-x".as_ref()][..]
                .iter()
                .copied()
                .chain(is_zip.then_some("-k".as_ref()))
                .chain([archive.as_os_str(), apple_out.as_os_str()])
                .collect::<Vec<_>>());
            let native_out = temp.path().join(format!("native-{name}"));
            if is_zip {
                extract_zip(&archive, &native_out, Limits::default()).unwrap();
            } else {
                extract_cpio(&archive, &native_out, Limits::default()).unwrap();
            }
            assert_eq!(
                manifest(&native_out).unwrap(),
                manifest(&apple_out).unwrap(),
                "{name}"
            );
        }
    }
}
