//! Builds packages in-process, for Linux and `RUSSET_NATIVE=pkgbuild`.
//!
//! It accepts the same requests as `autopkgserver` and reproduces its
//! result without running as root: ownership and modes that the helper sets
//! on a copy of the root are recorded straight into the payload and BOM.
//!
//! - The payload root is `root:admin` (0:80) with mode `01775`, as the
//!   helper's temporary root is.
//! - Other entries keep their modes. Entries owned by the invoking user are
//!   recorded as `RUSSET_PKG_OWNER` (default `501:20`, a typical macOS user),
//!   since the helper's root copy keeps the caller's owner.
//! - `chown` entries apply in order: a file gets the owner and mode; a
//!   folder gets the owner on itself and everything in it, and the mode on
//!   everything in it but not itself.
//! - User and group names resolve through macOS's standard accounts, not
//!   the Linux host's.

use crate::common::{octal, string};
use crate::packager::{chown_path, verify_basic};
use crate::runtime::packaging_syntax;
use plist::{Dictionary, Value};
use russet_pkgbuild::{Node, Options};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The environment variable naming the owner recorded for the invoking
/// user's files, as `uid:gid`.
const OWNER_VARIABLE: &str = "RUSSET_PKG_OWNER";

/// Standard macOS users and groups (from `dscl . -list /Users UniqueID` and
/// `/Groups PrimaryGroupID`), so packages built on Linux record the IDs a Mac
/// assigns these names.
pub(crate) const USERS: [(&str, u32); 6] = [
    ("root", 0),
    ("daemon", 1),
    ("_unknown", 99),
    ("_www", 70),
    ("nobody", 4_294_967_294),
    ("_installer", 96),
];
pub(crate) const GROUPS: [(&str, u32); 17] = [
    ("wheel", 0),
    ("daemon", 1),
    ("kmem", 2),
    ("sys", 3),
    ("tty", 4),
    ("operator", 5),
    ("mail", 6),
    ("bin", 7),
    ("procview", 8),
    ("procmod", 9),
    ("owner", 10),
    ("everyone", 12),
    ("staff", 20),
    ("_www", 70),
    ("admin", 80),
    ("nobody", 4_294_967_294),
    ("nogroup", 4_294_967_295),
];

fn mac_id(value: &Value, group: bool) -> Result<u32, String> {
    let kind = if group { "group" } else { "user" };
    match value {
        Value::Integer(number) => number
            .as_unsigned()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| format!("Invalid {kind} id {number}")),
        Value::Boolean(value) => Ok(u32::from(*value)),
        Value::String(name) => (if group { &GROUPS[..] } else { &USERS[..] })
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
            .ok_or_else(|| format!("Unknown chown {kind} {name}")),
        _ => Err(format!("Invalid {kind} id")),
    }
}

fn owner() -> Result<(u32, u32), String> {
    match std::env::var(OWNER_VARIABLE) {
        Err(_) => Ok((501, 20)),
        Ok(value) => value
            .split_once(':')
            .and_then(|(u, g)| Some((u.parse().ok()?, g.parse().ok()?)))
            .ok_or_else(|| format!("{OWNER_VARIABLE} must be uid:gid, such as 501:20")),
    }
}

/// The `pkgbuild_args` the native builder understands.
#[derive(Default)]
struct Arguments {
    install_location: Option<String>,
    min_os_version: Option<String>,
    filters: Vec<regex::Regex>,
}

fn arguments(request: &Dictionary) -> Result<Arguments, String> {
    let mut parsed = Arguments::default();
    let values: Vec<String> = request
        .get("pkgbuild_args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_string().map(str::to_owned))
        .collect();
    let mut values = values.into_iter();
    while let Some(flag) = values.next() {
        let (name, inline) = match flag.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (flag.clone(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| values.next())
                .ok_or_else(|| format!("pkgbuild argument {name} needs a value"))
        };
        match name.as_str() {
            "--install-location" => parsed.install_location = Some(value()?),
            "--min-os-version" => parsed.min_os_version = Some(value()?),
            "--filter" => parsed.filters.push(
                regex::Regex::new(&value()?)
                    .map_err(|e| format!("Invalid pkgbuild --filter: {e}"))?,
            ),
            "--compression" => {
                let mode = value()?;
                if mode != "legacy" {
                    return Err(format!(
                        "The native package builder supports only --compression legacy, not {mode}"
                    ));
                }
            }
            other => {
                return Err(format!(
                    "The native package builder doesn't support the pkgbuild argument {other}"
                ))
            }
        }
    }
    Ok(parsed)
}

/// Applies the request's `chown` entries to the nodes, as the helper applies
/// them to its copy of the root.
fn apply_chown(request: &Dictionary, root: &Path, nodes: &mut [Node]) -> Result<(), String> {
    let entries = request
        .get("chown")
        .and_then(Value::as_array)
        .ok_or("chown must be an array")?;
    for entry in entries {
        let entry = entry
            .as_dictionary()
            .ok_or("chown entry isn't dictionary")?;
        let path = chown_path(root, string(entry, "path")?)?;
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let uid = mac_id(
            entry.get("user").ok_or("chown entry is missing user")?,
            false,
        )?;
        let gid = mac_id(
            entry.get("group").ok_or("chown entry is missing group")?,
            true,
        )?;
        let mode = entry
            .get("mode")
            .map(|v| {
                v.as_string()
                    .ok_or("chown mode must be a string")
                    .and_then(|s| octal(s).map_err(|_| "Invalid chown mode"))
            })
            .transpose()?;
        let is_file = path.is_file();
        for node in nodes.iter_mut() {
            let itself = node.path == relative;
            let inside =
                !is_file && (relative.is_empty() || node.path.starts_with(&format!("{relative}/")));
            if itself || inside {
                node.uid = uid;
                node.gid = gid;
                if let Some(mode) = mode {
                    if is_file || inside {
                        node.mode = (mode & 0o7777) as u16;
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn package(request: &Dictionary) -> Result<String, String> {
    let mut request = request.clone();
    let (valid, errors) = packaging_syntax(&mut request);
    if !valid {
        return Err(errors.join(", "));
    }
    let root = PathBuf::from(string(&request, "pkgroot")?);
    if !root.exists() {
        return Err("Can't find pkgroot".into());
    }
    // SAFETY: getuid and getgid have no preconditions and can't fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    verify_basic(&request, uid)?;
    let arguments = arguments(&request)?;
    let (owner_uid, owner_gid) = owner()?;

    let mut nodes =
        russet_pkgbuild::collect(&root, &arguments.filters).map_err(|e| e.to_string())?;
    for node in &mut nodes {
        if node.path.is_empty() {
            (node.uid, node.gid, node.mode) = (0, 80, 0o1775);
            continue;
        }
        if node.uid == uid {
            node.uid = owner_uid;
        }
        if node.gid == gid {
            node.gid = owner_gid;
        }
    }
    apply_chown(&request, &root, &mut nodes)?;
    let components = russet_pkgbuild::analyze(&root).map_err(|e| e.to_string())?;

    let name = format!("{}.pkg", string(&request, "pkgname")?);
    let directory = PathBuf::from(string(&request, "pkgdir")?);
    let output = directory.join(&name);
    if let Ok(metadata) = fs::symlink_metadata(&output) {
        if metadata.uid() != uid {
            return Err(format!(
                "Existing pkg {} not owned by {uid}",
                output.display()
            ));
        }
        crate::common::remove(&output)
            .map_err(|e| format!("Can't remove existing pkg {}: {e}", output.display()))?;
    }
    let scripts = string(&request, "scripts")?;
    let info = string(&request, "infofile")?;
    let staging = tempfile::Builder::new()
        .prefix("autopkgtmp-")
        .suffix(&format!("-{name}"))
        .tempfile_in(&directory)
        .map_err(|e| e.to_string())?
        .into_temp_path();
    russet_pkgbuild::build(
        &nodes,
        &Options {
            identifier: string(&request, "id")?,
            version: string(&request, "version")?,
            install_location: arguments.install_location.as_deref(),
            min_os_version: arguments.min_os_version.as_deref(),
            scripts: (!scripts.is_empty()).then(|| Path::new(scripts)),
            info_template: (!info.is_empty()).then(|| Path::new(info)),
            components: &components,
        },
        &staging,
    )
    .map_err(|e| e.to_string())?;
    staging.persist(&output).map_err(|e| e.to_string())?;
    Ok(output.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(root: &Path, dir: &Path, chown: Vec<Value>) -> Dictionary {
        Dictionary::from_iter([
            (
                "pkgroot",
                Value::String(root.to_string_lossy().into_owned()),
            ),
            ("pkgdir", Value::String(dir.to_string_lossy().into_owned())),
            ("pkgname", Value::String("Tool-1.0".into())),
            ("pkgtype", Value::String("flat".into())),
            ("id", Value::String("com.example.tool".into())),
            ("version", Value::String("1.0".into())),
            ("infofile", Value::String(String::new())),
            ("scripts", Value::String(String::new())),
            ("chown", Value::Array(chown)),
        ])
    }

    fn chown(path: &str, user: &str, group: &str, mode: Option<&str>) -> Value {
        let mut entry = Dictionary::from_iter([
            ("path", Value::String(path.into())),
            ("user", Value::String(user.into())),
            ("group", Value::String(group.into())),
        ]);
        if let Some(mode) = mode {
            entry.insert("mode".into(), Value::String(mode.into()));
        }
        Value::Dictionary(entry)
    }

    #[test]
    fn builds_with_helper_ownership_rules() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("payload");
        fs::create_dir_all(root.join("Applications/Tool.app/Contents")).unwrap();
        fs::write(
            root.join("Applications/Tool.app/Contents/Info.plist"),
            "<plist/>",
        )
        .unwrap();
        fs::create_dir_all(root.join("Library/Thing")).unwrap();
        fs::write(root.join("Library/Thing/file"), "x").unwrap();
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        let request = request(
            &root,
            &out,
            vec![
                chown("Applications", "root", "admin", None),
                chown("Library", "root", "wheel", Some("0755")),
            ],
        );
        let path = package(&request).unwrap();
        assert_eq!(path, out.join("Tool-1.0.pkg").to_string_lossy());
        let mut archive = russet_xar::Archive::open(Path::new(&path)).unwrap();
        let bom = russet_mkbom::read(&archive.read("Bom", 1 << 20).unwrap()).unwrap();
        let line = |p: &str| bom.iter().find(|e| e.path == p).unwrap().lsbom_line();
        assert_eq!(bom[0].lsbom_line(), ".\t41775\t0/80");
        assert!(line("Applications").ends_with("\t0/80"));
        assert!(line("Applications/Tool.app/Contents/Info.plist").contains("\t0/80\t"));
        // The folder's own mode isn't changed; its contents' modes are.
        let library = bom.iter().find(|e| e.path == "Library").unwrap();
        assert_eq!((library.uid, library.gid), (0, 0));
        assert_eq!(
            bom.iter()
                .find(|e| e.path == "Library/Thing/file")
                .unwrap()
                .mode,
            0o755
        );
        // Rebuilding replaces the existing package.
        package(&request).unwrap();
    }

    #[test]
    fn rejects_unknown_owners_and_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("payload");
        fs::create_dir_all(root.join("Applications")).unwrap();
        let error = package(&request(
            &root,
            temp.path(),
            vec![chown("Applications", "nosuchuser", "admin", None)],
        ))
        .unwrap_err();
        assert!(error.contains("Unknown chown user nosuchuser"), "{error}");
        let mut with_sign = request(&root, temp.path(), vec![]);
        with_sign.insert(
            "pkgbuild_args".into(),
            Value::Array(vec![
                Value::String("--sign".into()),
                Value::String("Developer ID".into()),
            ]),
        );
        let error = package(&with_sign).unwrap_err();
        assert!(
            error.contains("doesn't support the pkgbuild argument --sign"),
            "{error}"
        );
    }

    /// The fixed account table matches this Mac's.
    #[cfg(target_os = "macos")]
    #[test]
    fn account_table_matches_macos() {
        for (name, id) in USERS {
            assert_eq!(
                crate::common::numeric_id(&Value::String(name.into()), false, false).unwrap(),
                id,
                "{name}"
            );
        }
        for (name, id) in GROUPS {
            assert_eq!(
                crate::common::numeric_id(&Value::String(name.into()), true, false).unwrap(),
                id,
                "{name}"
            );
        }
    }
}
