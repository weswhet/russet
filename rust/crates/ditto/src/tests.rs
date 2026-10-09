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
fn zip_reads_unflagged_names_as_utf8_like_ditto() {
    let temp = tempfile::tempdir().unwrap();
    let archive = temp.path().join("names.zip");
    zip_with(&archive, |z| {
        z.start_file("Scripts/👋 About.rtf", options(0o644))
            .unwrap();
        z.write_all(b"hello").unwrap();
    });
    // Clear the UTF-8 flag (bit 11) in the local and central headers, as
    // macOS zip tools leave it.
    let mut bytes = fs::read(&archive).unwrap();
    for (signature, offset) in [(b"PK\x03\x04", 6), (b"PK\x01\x02", 8)] {
        let start = bytes.windows(4).position(|w| w == signature).unwrap();
        bytes[start + offset + 1] &= !0x08;
    }
    fs::write(&archive, bytes).unwrap();
    let out = temp.path().join("out");
    extract_zip(&archive, &out, Limits::default()).unwrap();
    assert_eq!(
        fs::read(out.join("Scripts/👋 About.rtf")).unwrap(),
        b"hello"
    );
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
/// macOS `tar -x` merges `._name` members into `name`'s attributes, so a
/// signed app archived with macOS `tar` (QuickBooks) extracts without
/// stray files its seal doesn't list.
#[test]
fn tar_merges_apple_double_members_like_macos_tar() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut add = |path: &str, kind: tar::EntryType, mode: u32, data: &[u8], link: Option<&str>| {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_mode(mode);
        header.set_size(data.len() as u64);
        if let Some(link) = link {
            header.set_link_name(link).unwrap();
        }
        builder.append_data(&mut header, path, data).unwrap();
    };
    add(
        "App.app/Contents/",
        tar::EntryType::Directory,
        0o755,
        b"",
        None,
    );
    // bsdtar writes the metadata right before the entry it describes.
    let double = apple_double([0; 32], &[("com.example.test", b"hello")], b"");
    add(
        "App.app/Contents/._Frameworks",
        tar::EntryType::Regular,
        0o644,
        &double,
        None,
    );
    add(
        "App.app/Contents/Frameworks/",
        tar::EntryType::Directory,
        0o700,
        b"",
        None,
    );
    add(
        "App.app/Contents/MacOS/App",
        tar::EntryType::Regular,
        0o755,
        b"binary",
        None,
    );
    add(
        "App.app/Contents/Current",
        tar::EntryType::Symlink,
        0o755,
        b"",
        Some("MacOS"),
    );
    add(
        "App.app/Contents/MacOS/Copy",
        tar::EntryType::Link,
        0o755,
        b"",
        Some("App.app/Contents/MacOS/App"),
    );
    add(
        "App.app/._notes",
        tar::EntryType::Regular,
        0o644,
        b"plain text",
        None,
    );
    let archive = builder.into_inner().unwrap();

    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    crate::extract_tar(archive.as_slice(), &out, Limits::default()).unwrap();
    let entries = manifest(&out).unwrap();
    let find = |p: &str| {
        entries
            .iter()
            .find(|e| e.path == p)
            .unwrap_or_else(|| panic!("{p}"))
    };
    assert!(entries.iter().all(|e| !e.path.ends_with("._Frameworks")));
    let frameworks = find("App.app/Contents/Frameworks");
    assert_eq!(frameworks.mode, 0o700);
    assert!(frameworks.xattrs.contains_key("com.example.test"));
    assert_eq!(find("App.app/Contents/MacOS/App").mode, 0o755);
    assert_eq!(
        fs::read(out.join("App.app/Contents/MacOS/Copy")).unwrap(),
        b"binary"
    );
    let link = find("App.app/Contents/Current");
    assert_eq!(
        (link.kind, link.target.as_deref()),
        (EntryKind::Symlink, Some("MacOS"))
    );
    assert_eq!(find("App.app/._notes").size, Some(10));
}

#[test]
fn tar_rejects_escapes() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_mode(0o644);
    // append_data refuses `..`, so write the name directly.
    header.as_gnu_mut().unwrap().name[..9].copy_from_slice(b"../escape");
    header.set_cksum();
    builder.append(&header, &b"x"[..]).unwrap();
    let archive = builder.into_inner().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let out = temp.path().join("out");
    assert!(crate::extract_tar(archive.as_slice(), &out, Limits::default()).is_err());
    assert!(!temp.path().join("escape").exists());
}

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

#[test]
fn cpio_writer_round_trips() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    fs::create_dir_all(src.join("bin")).unwrap();
    fs::write(src.join("bin/tool"), "tool").unwrap();
    fs::set_permissions(src.join("bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::hard_link(src.join("bin/tool"), src.join("bin/alias")).unwrap();
    std::os::unix::fs::symlink("tool", src.join("bin/link")).unwrap();
    let archive = temp.path().join("tree.cpio.gz");
    let encoder = flate2::write::GzEncoder::new(
        fs::File::create(&archive).unwrap(),
        flate2::Compression::default(),
    );
    crate::write_tree(&src, encoder, |_, m| crate::Header {
        mode: m.mode(),
        uid: 0,
        gid: 80,
        mtime: m.mtime() as u64,
        ino: 0,
        nlink: m.nlink() as u32,
    })
    .unwrap()
    .finish()
    .unwrap();
    let out = temp.path().join("out");
    extract_cpio(&archive, &out, Limits::default()).unwrap();
    assert_eq!(manifest(&out).unwrap(), manifest(&src).unwrap());
    #[cfg(target_os = "macos")]
    {
        let apple = temp.path().join("apple");
        let status = std::process::Command::new("/usr/bin/ditto")
            .args(["-x"])
            .arg(&archive)
            .arg(&apple)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(manifest(&apple).unwrap(), manifest(&src).unwrap());
    }
}

/// Packages built with `pkgbuild --compression latest` have pbzx payloads,
/// which `ditto` can't read; compare with `aa extract`, which can.
#[cfg(target_os = "macos")]
#[test]
fn pbzx_payload_matches_aa() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root/App.app/Contents/MacOS");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("App"), "binary".repeat(1000)).unwrap();
    fs::set_permissions(root.join("App"), fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("MacOS", temp.path().join("root/App.app/Contents/Link")).unwrap();
    let t = |p: &str| temp.path().join(p);
    let ok = |c: &mut Command| assert!(c.status().unwrap().success(), "{c:?}");
    ok(Command::new("/usr/bin/pkgbuild")
        .args(["--quiet", "--root"])
        .arg(t("root"))
        .args([
            "--identifier",
            "com.example.p",
            "--version",
            "1",
            "--min-os-version",
            "12.0",
            "--compression",
            "latest",
        ])
        .arg(t("p.pkg")));
    fs::create_dir(t("x")).unwrap();
    ok(Command::new("/usr/bin/xar")
        .arg("-xf")
        .arg(t("p.pkg"))
        .arg("-C")
        .arg(t("x")));
    fs::create_dir(t("apple")).unwrap();
    ok(Command::new("/usr/bin/aa")
        .args(["extract", "-i"])
        .arg(t("x/Payload"))
        .arg("-d")
        .arg(t("apple")));
    extract_cpio(&t("x/Payload"), &t("native"), Limits::default()).unwrap();
    let strip = |p: &Path| -> Vec<_> {
        manifest(p)
            .unwrap()
            .into_iter()
            .map(|mut e| {
                e.xattrs.clear();
                e
            })
            .collect()
    };
    assert_eq!(strip(&t("native")), strip(&t("apple")));
}

/// Payloads that are Apple Archives, plain or inside pbzx (as `aa` writes
/// LZMA archives), extract and read like `aa extract`.
#[test]
fn apple_archive_payloads() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../aa/tests/fixtures");
    let expected: serde_json::Value =
        serde_json::from_slice(&fs::read(fixtures.join("expected.json")).unwrap()).unwrap();
    for name in ["lzfse.aar", "lzma.aar", "raw.aar"] {
        let out = tempfile::tempdir().unwrap();
        extract_cpio(&fixtures.join(name), out.path(), Limits::default()).unwrap();
        assert_eq!(
            serde_json::to_value(manifest(out.path()).unwrap()).unwrap(),
            expected,
            "{name}"
        );
        let member = crate::read_cpio_member(
            &fixtures.join(name),
            Path::new("./App.app/Contents/Info.plist"),
            1 << 20,
        )
        .unwrap();
        assert_eq!(member.as_deref(), Some(&b"<plist/>\n"[..]), "{name}");
    }
}
