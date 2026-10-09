//! `cargo xtask package`: archive native development binaries for one target.

use crate::archive::{is_apple, is_windows, verify_binary, write_archive, Entries};
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The repository root, two levels above this crate.
pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

fn licenses_below(
    directory: &Path,
    relative: &Path,
    found: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut children: Vec<_> = fs::read_dir(directory.join(relative))
        .map_err(|error| format!("{}: {error}", directory.join(relative).display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    children.sort_by_key(|child| child.file_name());
    for child in children {
        let path = relative.join(child.file_name());
        let full = directory.join(&path);
        if full.is_dir() {
            licenses_below(directory, &path, found)?;
        } else if full.is_file() && child.file_name().to_string_lossy().starts_with("LICENSE") {
            found.push(path);
        }
    }
    Ok(())
}

/// Collect the archive members for `target`: the `russet` executable from
/// `bin_dir`, the matching installer, launchd jobs on macOS, and every
/// license and notice the binaries need.
pub fn entries(root: &Path, target: &str, bin_dir: &Path) -> Result<Entries, String> {
    let suffix = if is_windows(target) { ".exe" } else { "" };
    let mut entries = Entries::new();
    let data = read(&bin_dir.join(format!("russet{suffix}")))?;
    verify_binary(&data, target)?;
    entries.insert(format!("bin/russet{suffix}"), (data, 0o755));
    entries.insert(
        "LICENSE.txt".into(),
        (read(&root.join("LICENSE.txt"))?, 0o644),
    );
    let certifi = root.join("rust/crates/processors/data/certifi");
    for notice in ["LICENSE", "README.md"] {
        entries.insert(
            format!("licenses/certifi/{notice}"),
            (read(&certifi.join(notice))?, 0o644),
        );
    }
    let tls = root.join("rust/crates/processors/data/tls-licenses");
    let mut found = Vec::new();
    licenses_below(&tls, Path::new(""), &mut found)?;
    for notice in found {
        let name = notice
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        entries.insert(
            format!("licenses/tls/{name}"),
            (read(&tls.join(&notice))?, 0o644),
        );
    }
    let fancy_regex = root.join("rust/vendor/fancy-regex");
    for notice in [
        "LICENSE",
        "LICENSE-PYTHON",
        "LICENSE-UNICODE",
        "AUTOPKG-PATCH.md",
    ] {
        entries.insert(
            format!("licenses/fancy-regex/{notice}"),
            (read(&fancy_regex.join(notice))?, 0o644),
        );
    }
    for (name, bytes) in crate::licenses::shipped(root)? {
        entries.insert(name, (bytes, 0o644));
    }
    let distribution = root.join("rust/distribution");
    entries.insert(
        "README.md".into(),
        (read(&root.join("rust/README.md"))?, 0o644),
    );
    entries.insert(
        "INSTALL.md".into(),
        (read(&distribution.join("INSTALL.md"))?, 0o644),
    );
    if is_windows(target) {
        entries.insert(
            "install.ps1".into(),
            (read(&distribution.join("install.ps1"))?, 0o644),
        );
    } else {
        entries.insert(
            "install.sh".into(),
            (read(&distribution.join("install.sh"))?, 0o755),
        );
    }
    if is_apple(target) {
        for (name, label) in LAUNCHD_JOBS {
            let definition = distribution.join("launchd").join(format!("{label}.json"));
            entries.insert(
                format!("launchd/{name}"),
                (launchd_plist(&definition, label)?, 0o644),
            );
        }
    }
    Ok(entries)
}

/// The archive's launchd plists for install.sh, and the job definitions in
/// `rust/distribution/launchd` they come from. `russet --install-helpers`
/// installs the same definitions.
const LAUNCHD_JOBS: [(&str, &str); 2] = [
    ("russet-server.plist", "com.github.weswhet.russet.server"),
    (
        "russet-installd.plist",
        "com.github.weswhet.russet.installd",
    ),
];

/// Convert a JSON job definition to an XML plist.
fn launchd_plist(path: &Path, label: &str) -> Result<Vec<u8>, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let job: plist::Value = serde_json::from_str(&text)
        .map_err(|error| format!("{}: invalid JSON: {error}", path.display()))?;
    let found = job
        .as_dictionary()
        .and_then(|job| job.get("Label"))
        .and_then(plist::Value::as_string);
    if found != Some(label) {
        return Err(format!("{}: Label must be {label}", path.display()));
    }
    let mut xml = Vec::new();
    job.to_writer_xml(&mut xml)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(xml)
}

/// Write `russet-development-TARGET.tar.gz` (or `.zip` for Windows) to
/// `output` and return its path.
pub fn package(
    root: &Path,
    target: &str,
    bin_dir: &Path,
    output: &Path,
) -> Result<PathBuf, String> {
    let entries = entries(root, target, bin_dir)?;
    let archive_name = format!("russet-development-{target}");
    fs::create_dir_all(output).map_err(|error| format!("{}: {error}", output.display()))?;
    let path = output.join(format!(
        "{archive_name}{}",
        if is_windows(target) {
            ".zip"
        } else {
            ".tar.gz"
        }
    ));
    write_archive(&path, &archive_name, &entries)?;
    Ok(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::archive::{read_tar_gz, read_zip, TARGETS};

    /// A minimal header that satisfies the packager's architecture check.
    /// These fixtures are never executed.
    pub(crate) fn executable(target: &str) -> Vec<u8> {
        let mut data = vec![0u8; 128];
        if is_apple(target) {
            data[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
            let architecture: u32 = if target.starts_with("aarch64") {
                0x0100_000C
            } else {
                0x0100_0007
            };
            data[4..8].copy_from_slice(&architecture.to_le_bytes());
        } else if target.contains("linux") {
            data[..6].copy_from_slice(b"\x7fELF\x02\x01");
            data[18..20].copy_from_slice(&62u16.to_le_bytes());
        } else {
            data[..2].copy_from_slice(b"MZ");
            data[60..64].copy_from_slice(&64u32.to_le_bytes());
            data[64..68].copy_from_slice(b"PE\0\0");
            data[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        }
        data
    }

    #[test]
    fn target_archives_include_correct_installer_and_permissions() {
        let root = repository_root();
        for (target, _, _) in TARGETS {
            let directory = tempfile::tempdir().unwrap();
            let bins = directory.path().join("bin");
            fs::create_dir(&bins).unwrap();
            let suffix = if is_windows(target) { ".exe" } else { "" };
            fs::write(bins.join(format!("russet{suffix}")), executable(target)).unwrap();
            let output = directory.path().join("archives");
            let archive = package(&root, target, &bins, &output).unwrap();
            assert_eq!(fs::read_dir(&output).unwrap().count(), 1, "{target}");
            let data = fs::read(&archive).unwrap();
            let members = if suffix.is_empty() {
                read_tar_gz(&data).unwrap()
            } else {
                read_zip(&data).unwrap()
            };
            let prefix = format!("russet-development-{target}/");
            let entries: Entries = members
                .into_iter()
                .map(|m| {
                    assert!(m.regular, "{target}: {}", m.name);
                    (
                        m.name.strip_prefix(&prefix).unwrap().to_owned(),
                        (m.data, m.mode),
                    )
                })
                .collect();
            let installer = if suffix.is_empty() {
                "install.sh"
            } else {
                "install.ps1"
            };
            assert_eq!(
                entries[installer].0,
                fs::read(root.join("rust/distribution").join(installer)).unwrap(),
                "{target}"
            );
            assert_eq!(
                entries[installer].1,
                if suffix.is_empty() { 0o755 } else { 0o644 }
            );
            assert_eq!(
                entries[&format!("bin/russet{suffix}")],
                (executable(target), 0o755)
            );
            assert!(entries.contains_key("INSTALL.md"));
            assert!(entries.keys().any(|name| name.starts_with("licenses/tls/")));
            assert!(entries.contains_key("licenses/third-party/README.md"));
            if is_apple(target) {
                for ((name, label), flag) in LAUNCHD_JOBS.iter().zip(["--server", "--installd"]) {
                    let (xml, mode) = &entries[&format!("launchd/{name}")];
                    assert_eq!(*mode, 0o644, "{target}: {name}");
                    let job = plist::Value::from_reader_xml(xml.as_slice()).unwrap();
                    let job = job.as_dictionary().unwrap();
                    assert_eq!(job["Label"].as_string(), Some(*label));
                    let arguments: Vec<_> = job["ProgramArguments"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|argument| argument.as_string().unwrap())
                        .collect();
                    assert_eq!(arguments, ["/opt/russet/russet", flag], "{target}: {name}");
                }
                assert!(!entries.keys().any(|name| name.starts_with("bin/autopkg")));
            } else {
                assert!(!entries.keys().any(|name| name.starts_with("launchd/")));
                assert!(!entries.keys().any(|name| name.contains("russet-server")));
            }
        }
    }

    #[test]
    fn rejects_an_executable_for_another_target() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("russet"),
            executable("x86_64-apple-darwin"),
        )
        .unwrap();
        let error = package(
            &repository_root(),
            "x86_64-unknown-linux-gnu",
            directory.path(),
            &directory.path().join("out"),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Executable does not match target x86_64-unknown-linux-gnu"
        );
    }
}
