//! Compares native extraction with what macOS shows when `hdiutil` mounts
//! the same image.

use crate::{extract, image_info};
use russet_fs::{manifest, Limits};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(program: &str, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// A tree with the features disk images carry: modes, symlinks, hard links,
/// extended attributes, a resource fork, Finder info, and an app copied with
/// HFS+ transparent compression.
fn source_tree(root: &Path) {
    let app = root.join("App.app/Contents");
    fs::create_dir_all(app.join("MacOS")).unwrap();
    fs::create_dir_all(app.join("Resources")).unwrap();
    fs::create_dir_all(root.join("Docs")).unwrap();
    fs::write(app.join("MacOS/App"), "binary").unwrap();
    fs::set_permissions(app.join("MacOS/App"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(app.join("Info.plist"), "<plist/>".repeat(500)).unwrap();
    let random: Vec<u8> = (0..40_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    fs::write(root.join("Docs/random.bin"), &random).unwrap();
    fs::set_permissions(
        root.join("Docs/random.bin"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fs::hard_link(
        root.join("Docs/random.bin"),
        root.join("Docs/random-link.bin"),
    )
    .unwrap();
    fs::write(
        root.join("Docs/text.txt"),
        "compressible line\n".repeat(4000),
    )
    .unwrap();
    std::os::unix::fs::symlink("../../../Docs/text.txt", app.join("Resources/link.txt")).unwrap();
    xattr::set(root.join("Docs/text.txt"), "com.example.test", b"hello").unwrap();
    xattr::set(
        root.join("Docs/text.txt"),
        "com.apple.ResourceFork",
        b"RSRC",
    )
    .unwrap();
    let mut finder = [0u8; 32];
    finder[..8].copy_from_slice(b"TEXTttxt");
    xattr::set(
        root.join("Docs/random.bin"),
        "com.apple.FinderInfo",
        &finder,
    )
    .unwrap();
    run(
        "/usr/bin/ditto",
        &[
            "--hfsCompression",
            app.parent().unwrap().to_str().unwrap(),
            root.join("Compressed.app").to_str().unwrap(),
        ],
    );
}

/// Mounts `image` read-only and returns the manifest of each volume.
fn mounted(image: &Path) -> Vec<Vec<russet_fs::Entry>> {
    let plist = run(
        "/usr/bin/hdiutil",
        &[
            "attach",
            "-plist",
            "-readonly",
            "-nobrowse",
            "-mountrandom",
            "/tmp",
            image.to_str().unwrap(),
        ],
    );
    let points: Vec<PathBuf> = plist
        .lines()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0].contains("<key>mount-point</key>"))
        .map(|w| {
            PathBuf::from(
                w[1].trim()
                    .trim_start_matches("<string>")
                    .trim_end_matches("</string>"),
            )
        })
        .collect();
    let manifests = points.iter().map(|p| manifest(p).unwrap()).collect();
    run(
        "/usr/bin/hdiutil",
        &["detach", "-quiet", points[0].to_str().unwrap()],
    );
    manifests
}

#[test]
fn native_extraction_matches_mounted_images() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    source_tree(&src);
    let cases = [
        ("UDZO", "HFS+"),
        ("UDBZ", "HFS+"),
        ("ULFO", "HFS+"),
        ("ULMO", "HFS+"),
        ("UDRO", "HFS+"),
        ("UDZO", "HFSX"),
        ("UDZO", "APFS"),
    ];
    for (format, filesystem) in cases {
        let name = format!("{format}-{}", filesystem.replace('+', "P"));
        let image = temp.path().join(format!("{name}.dmg"));
        run(
            "/usr/bin/hdiutil",
            &[
                "create",
                "-quiet",
                "-fs",
                filesystem,
                "-format",
                format,
                "-volname",
                "Fixture",
                "-srcfolder",
                src.to_str().unwrap(),
                image.to_str().unwrap(),
            ],
        );
        assert_eq!(image_info(&image).unwrap().format, format, "{name}");
        let out = temp.path().join(format!("out-{name}"));
        fs::create_dir(&out).unwrap();
        let extraction = extract(&image, &out, Limits::default()).unwrap();
        let native: Vec<_> = extraction
            .volumes
            .iter()
            .map(|v| manifest(v).unwrap())
            .collect();
        assert_eq!(native, mounted(&image), "{name}");
    }

    // Converting to ADC exercises the legacy UDCO decoder.
    let udro = temp.path().join("UDRO-HFSP.dmg");
    let udco = temp.path().join("UDCO.dmg");
    run(
        "/usr/bin/hdiutil",
        &[
            "convert",
            "-quiet",
            udro.to_str().unwrap(),
            "-format",
            "UDCO",
            "-o",
            udco.to_str().unwrap(),
        ],
    );
    assert_eq!(image_info(&udco).unwrap().format, "UDCO");
    let out = temp.path().join("out-UDCO");
    fs::create_dir(&out).unwrap();
    let extraction = extract(&udco, &out, Limits::default()).unwrap();
    let native: Vec<_> = extraction
        .volumes
        .iter()
        .map(|v| manifest(v).unwrap())
        .collect();
    assert_eq!(native, mounted(&udco), "UDCO");
}

/// Rebuilds the committed fixtures that the portable test checks on Linux:
/// small images in each supported format, with the manifest of each mounted
/// volume. Run on macOS with
/// `cargo test -p russet-hdiutil -- --ignored regenerate_fixtures`.
#[test]
#[ignore]
fn regenerate_fixtures() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let _ = fs::remove_dir_all(&fixtures);
    fs::create_dir_all(&fixtures).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("src");
    source_tree(&src);
    let cases = [
        ("UDZO", "HFS+"),
        ("UDBZ", "HFS+"),
        ("ULFO", "HFS+"),
        ("ULMO", "HFS+"),
        ("UDRO", "HFS+"),
        ("UDZO", "HFSX"),
        ("UDZO", "APFS"),
    ];
    for (format, filesystem) in cases {
        let name = format!("{format}-{}", filesystem.replace('+', "P"));
        let image = fixtures.join(format!("{name}.dmg"));
        run(
            "/usr/bin/hdiutil",
            &[
                "create",
                "-quiet",
                "-fs",
                filesystem,
                "-format",
                format,
                "-volname",
                "Fixture",
                "-srcfolder",
                src.to_str().unwrap(),
                image.to_str().unwrap(),
            ],
        );
        write_expected(&fixtures, &name, &image);
    }
    let udro = fixtures.join("UDRO-HFSP.dmg");
    let udco = fixtures.join("UDCO-HFSP.dmg");
    run(
        "/usr/bin/hdiutil",
        &[
            "convert",
            "-quiet",
            udro.to_str().unwrap(),
            "-format",
            "UDCO",
            "-o",
            udco.to_str().unwrap(),
        ],
    );
    write_expected(&fixtures, "UDCO-HFSP", &udco);
}

fn write_expected(fixtures: &Path, name: &str, image: &Path) {
    let expected = serde_json::json!({
        "format": image_info(image).unwrap().format,
        "volumes": mounted(image),
    });
    fs::write(
        fixtures.join(format!("{name}.json")),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
}

/// Apple's tools accept natively created images: `hdiutil` reports the
/// format and mounts the same files, and `fsck_hfs` finds no problems.
#[test]
fn hdiutil_accepts_created_images() {
    use crate::{create, CreateOptions};
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("Source");
    source_tree(&src);
    for (format, filesystem) in [
        ("UDZO", "HFS+"),
        ("ULFO", "Journaled HFS+"),
        ("UDRO", "HFS+"),
        ("ULFO", "APFS"),
    ] {
        let image = temp
            .path()
            .join(format!("{format}-{}.dmg", filesystem.len()));
        create(
            &src,
            &image,
            &CreateOptions {
                filesystem,
                format,
                zlib_level: 5,
                megabytes: None,
            },
        )
        .unwrap();
        let info = run(
            "/usr/bin/hdiutil",
            &["imageinfo", "-plist", image.to_str().unwrap()],
        );
        assert!(
            info.contains(&format!("<string>{format}</string>")),
            "{format}: {info}"
        );
        run("/usr/bin/hdiutil", &["verify", image.to_str().unwrap()]);
        let mounted = mounted(&image);
        // Extended attributes aren't copied, and owners are the mounting user.
        let strip = |entries: Vec<russet_fs::Entry>| -> Vec<_> {
            entries
                .into_iter()
                .map(|e| (e.path, e.kind, e.mode, e.sha256, e.target))
                .collect()
        };
        assert_eq!(
            strip(mounted[0].clone()),
            strip(manifest(&src).unwrap()),
            "{format}"
        );
        let attached = run(
            "/usr/bin/hdiutil",
            &["attach", "-nomount", image.to_str().unwrap()],
        );
        // `attach` first prints the checksum it verified.
        let devices: Vec<&str> = attached
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|d| d.starts_with("/dev/"))
            .collect();
        let device = devices.last().unwrap().to_string();
        let fsck = Command::new("/sbin/fsck_hfs")
            .args(["-n", &device])
            .output()
            .unwrap();
        run("/usr/bin/hdiutil", &["detach", "-quiet", devices[0]]);
        assert!(
            fsck.status.success(),
            "{format}: {}",
            String::from_utf8_lossy(&fsck.stdout)
        );
    }
}

/// Compares native extraction of the images listed in
/// `RUSSET_COMPARE_IMAGES` (separated by `:`) with the same images mounted
/// by macOS: `RUSSET_COMPARE_IMAGES=a.dmg:b.dmg cargo test -p russet-hdiutil
/// -- --ignored compare_downloaded_images`. For checking real downloads. It
/// compares the root listing, then every visible top-level item; system
/// folders such as `.Trashes` aren't readable on either side.
#[test]
#[ignore]
fn compare_downloaded_images() {
    let images = std::env::var("RUSSET_COMPARE_IMAGES").unwrap_or_default();
    let names = |root: &Path| -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    for image in images.split(':').filter(|i| !i.is_empty()) {
        let image = Path::new(image);
        let out = tempfile::tempdir().unwrap();
        let extraction = extract(image, out.path(), Limits::default()).unwrap();
        let mount = tempfile::tempdir().unwrap();
        // Accept any license agreement, as AutoPkg does.
        let mut attach = Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noverify",
                "-mountpoint",
            ])
            .arg(mount.path())
            .arg(image)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        use std::io::Write;
        let _ = attach.stdin.take().unwrap().write_all(b"Y\n");
        assert!(attach.wait().unwrap().success(), "{}", image.display());
        let native = &extraction.volumes[0];
        let result = std::panic::catch_unwind(|| {
            assert_eq!(names(native), names(mount.path()), "{}", image.display());
            for name in names(mount.path()).iter().filter(|n| !n.starts_with('.')) {
                let (ours, apple) = (native.join(name), mount.path().join(name));
                if apple.is_dir() && !apple.is_symlink() {
                    assert_eq!(
                        manifest(&ours).unwrap(),
                        manifest(&apple).unwrap(),
                        "{} {name}",
                        image.display()
                    );
                } else {
                    let xattrs = |p: &Path| {
                        let mut names: Vec<_> = xattr::list(p)
                            .unwrap()
                            .filter(|n| n != "com.apple.provenance" && n != "com.apple.quarantine")
                            .collect();
                        names.sort();
                        names
                    };
                    assert_eq!(fs::read(&ours).ok(), fs::read(&apple).ok(), "{name}");
                    assert_eq!(xattrs(&ours), xattrs(&apple), "{name}");
                }
            }
        });
        run(
            "/usr/bin/hdiutil",
            &["detach", "-force", mount.path().to_str().unwrap()],
        );
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
