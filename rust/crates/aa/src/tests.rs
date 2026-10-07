use crate::{extract, is_apple_archive, read_member};
use russet_fs::{manifest, Limits};
use std::fs;
use std::path::{Path, PathBuf};

const CODECS: [&str; 5] = ["lzfse", "zlib", "lzma", "lz4", "raw"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The extracted tree as JSON, the form the fixture expectations use.
fn tree(root: &Path) -> serde_json::Value {
    serde_json::to_value(manifest(root).unwrap()).unwrap()
}

#[test]
fn fixtures_match_aa_extract() {
    let expected: serde_json::Value =
        serde_json::from_slice(&fs::read(fixtures().join("expected.json")).unwrap()).unwrap();
    for codec in CODECS {
        let archive = fixtures().join(format!("{codec}.aar"));
        let out = tempfile::tempdir().unwrap();
        extract(&archive, out.path(), Limits::default()).unwrap();
        assert_eq!(tree(out.path()), expected, "{codec}");
        let member = read_member(&archive, Path::new("App.app/Contents/Info.plist"), 1 << 20)
            .unwrap()
            .unwrap();
        assert_eq!(member, b"<plist/>\n", "{codec}");
        assert_eq!(
            read_member(&archive, Path::new("missing"), 1 << 20).unwrap(),
            None
        );
    }
}

#[test]
fn rejects_bad_input_and_limits() {
    let temp = tempfile::tempdir().unwrap();
    let bogus = temp.path().join("bogus.aar");
    fs::write(&bogus, b"not an archive").unwrap();
    assert!(!is_apple_archive(b"not an archive"));
    assert!(extract(&bogus, &temp.path().join("out"), Limits::default()).is_err());

    let bitmap = temp.path().join("bitmap.aar");
    fs::write(&bitmap, b"pbzb\0\0\0\0\0\x40\0\0").unwrap();
    let error = extract(&bitmap, &temp.path().join("out"), Limits::default()).unwrap_err();
    assert!(error.to_string().contains("LZBITMAP"), "{error}");

    let limits = Limits {
        max_total_bytes: 1000,
        ..Limits::default()
    };
    let error = extract(
        &fixtures().join("lzfse.aar"),
        &temp.path().join("limited"),
        limits,
    )
    .unwrap_err();
    assert!(error.to_string().contains("limit"), "{error}");

    // A truncated archive fails instead of producing a partial tree silently.
    let whole = fs::read(fixtures().join("raw.aar")).unwrap();
    let truncated = temp.path().join("truncated.aar");
    fs::write(&truncated, &whole[..whole.len() - 100]).unwrap();
    assert!(extract(&truncated, &temp.path().join("t"), Limits::default()).is_err());
}

#[test]
fn rejects_paths_outside_the_destination() {
    let temp = tempfile::tempdir().unwrap();
    // One file entry, `PAT` = "../escape", with no data.
    let mut entry = b"AA01\0\0TYP1FPATP\x09\0../escape".to_vec();
    let size = entry.len() as u16;
    entry[4..6].copy_from_slice(&size.to_le_bytes());
    let archive = temp.path().join("escape.aar");
    fs::write(&archive, entry).unwrap();
    let out = temp.path().join("out");
    assert!(extract(&archive, &out, Limits::default()).is_err());
    assert!(!temp.path().join("escape").exists());
}

#[cfg(target_os = "macos")]
mod apple {
    use super::*;
    use std::process::Command;

    fn run(command: &mut Command) {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A tree with the entry kinds `aa` records: nested folders, modes, an
    /// empty file, a hard link, a symlink, an extended attribute, a
    /// decomposed name, and a file larger than the 64 KiB blocks the
    /// archives use, half of it incompressible.
    fn source_tree(root: &Path) {
        let contents = root.join("App.app/Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        fs::write(contents.join("Info.plist"), b"<plist/>\n").unwrap();
        let mut seed = 0x2545_f491_u32;
        let mut big: Vec<u8> = (0..40_000)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        big.extend(b"compressible text ".iter().cycle().take(60_000));
        let binary = contents.join("MacOS/App");
        fs::write(&binary, &big).unwrap();
        run(Command::new("/bin/chmod").arg("755").arg(&binary));
        fs::write(contents.join("empty"), b"").unwrap();
        fs::hard_link(&binary, contents.join("MacOS/link")).unwrap();
        std::os::unix::fs::symlink("Contents/MacOS/App", root.join("App.app/App")).unwrap();
        fs::write(root.join("Re\u{301}sume\u{301}.txt"), b"name\n").unwrap();
        run(Command::new("/usr/bin/xattr")
            .args(["-w", "com.example.test", "value"])
            .arg(contents.join("Info.plist")));
        fs::create_dir(root.join("locked")).unwrap();
        run(Command::new("/bin/chmod")
            .arg("555")
            .arg(root.join("locked")));
    }

    fn archive(source: &Path, out: &Path, codec: &str) {
        run(Command::new("/usr/bin/aa")
            .args(["archive", "-d"])
            .arg(source)
            .arg("-o")
            .arg(out)
            .args(["-a", codec, "-b", "64k"]));
    }

    fn aa_extract(archive: &Path, out: &Path) {
        fs::create_dir_all(out).unwrap();
        run(Command::new("/usr/bin/aa")
            .args(["extract", "-i"])
            .arg(archive)
            .arg("-d")
            .arg(out));
    }

    /// Every codec extracts to the tree `aa extract` produces, including
    /// archives `aa` writes from a fresh tree on this macOS version.
    #[test]
    fn matches_aa_extract() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        source_tree(&source);
        for codec in CODECS {
            let file = temp.path().join(format!("{codec}.aar"));
            archive(&source, &file, codec);
            let apple = temp.path().join(format!("apple-{codec}"));
            aa_extract(&file, &apple);
            let native = temp.path().join(format!("native-{codec}"));
            extract(&file, &native, Limits::default()).unwrap();
            assert_eq!(tree(&native), tree(&apple), "{codec}");
        }
        let file = temp.path().join("lzbitmap.aar");
        archive(&source, &file, "lzbitmap");
        let error = extract(&file, &temp.path().join("x"), Limits::default()).unwrap_err();
        assert!(error.to_string().contains("LZBITMAP"), "{error}");
    }

    /// Rebuilds `tests/fixtures` with `aa`:
    /// `cargo test -p russet-aa -- --ignored regenerate_fixtures`.
    #[test]
    #[ignore]
    fn regenerate_fixtures() {
        let _ = fs::remove_dir_all(fixtures());
        fs::create_dir_all(fixtures()).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("src");
        source_tree(&source);
        let mut expected = None;
        for codec in CODECS {
            let file = fixtures().join(format!("{codec}.aar"));
            archive(&source, &file, codec);
            let apple = temp.path().join(format!("apple-{codec}"));
            aa_extract(&file, &apple);
            let manifest = tree(&apple);
            assert!(expected.get_or_insert(manifest.clone()) == &manifest);
        }
        let mut json = serde_json::to_vec_pretty(&expected.unwrap()).unwrap();
        json.push(b'\n');
        fs::write(fixtures().join("expected.json"), json).unwrap();
    }
}
