//! `cargo xtask licenses`: collect the license and notice files of every
//! third-party crate linked into the shipped binaries, for every release
//! target, into `rust/licenses/`. `--check` fails when that folder is stale.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The package whose binary ships. Its dependencies, such as the helper services, are included.
const ROOTS: [&str; 1] = ["russet"];

/// One third-party crate in the shipped binaries.
struct Crate {
    name: String,
    vendored: bool,
    version: String,
    license: String,
    repository: String,
    directory: PathBuf,
    license_file: Option<PathBuf>,
}

fn metadata(rust: &Path, target: &str) -> Result<Value, String> {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(rust)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            target,
        ])
        .output()
        .map_err(|error| format!("cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("cargo metadata: {error}"))
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// The third-party crates the shipped binaries link on `target`, following
/// normal dependencies only.
fn crates_for(rust: &Path, target: &str, out: &mut BTreeMap<String, Crate>) -> Result<(), String> {
    let metadata = metadata(rust, target)?;
    let members: BTreeSet<&str> = metadata["workspace_members"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let packages: HashMap<&str, &Value> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| (p["id"].as_str().unwrap_or_default(), p))
        .collect();
    let nodes: HashMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|n| (n["id"].as_str().unwrap_or_default(), n))
        .collect();
    let mut stack: Vec<&str> = packages
        .iter()
        .filter(|(id, p)| {
            members.contains(*id) && ROOTS.contains(&p["name"].as_str().unwrap_or(""))
        })
        .map(|(id, _)| *id)
        .collect();
    if stack.len() != ROOTS.len() {
        return Err("cargo metadata doesn't list the shipped packages".into());
    }
    let mut seen = BTreeSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = nodes
            .get(id)
            .ok_or_else(|| format!("cargo metadata has no resolve node for {id}"))?;
        for dep in node["deps"].as_array().into_iter().flatten() {
            let normal = dep["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|kind| kind["kind"].is_null());
            if normal {
                if let Some(pkg) = dep["pkg"].as_str() {
                    stack.push(pkg);
                }
            }
        }
        if members.contains(id) {
            continue;
        }
        let package = packages
            .get(id)
            .ok_or_else(|| format!("cargo metadata has no package {id}"))?;
        let manifest = PathBuf::from(text(package, "manifest_path"));
        let directory = manifest
            .parent()
            .ok_or("A package manifest has no folder")?
            .to_path_buf();
        let license_file = package["license_file"]
            .as_str()
            .map(|file| directory.join(file));
        let name = text(package, "name");
        let version = text(package, "version");
        // Crates vendored under rust/vendor have no registry source and may
        // share a version with a crates.io copy.
        let vendored = if package["source"].is_null() {
            "-vendored"
        } else {
            ""
        };
        out.insert(
            format!("{name}-{version}{vendored}"),
            Crate {
                name,
                vendored: !vendored.is_empty(),
                version,
                license: text(package, "license"),
                repository: text(package, "repository"),
                directory,
                license_file,
            },
        );
    }
    Ok(())
}

/// License and notice files at the top of a crate's folder.
fn notices(krate: &Crate) -> Result<Vec<PathBuf>, String> {
    let mut found: BTreeSet<PathBuf> = krate.license_file.iter().cloned().collect();
    let entries = fs::read_dir(&krate.directory)
        .map_err(|error| format!("{}: {error}", krate.directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_uppercase();
        let notice = [
            "LICENSE",
            "LICENCE",
            "COPYING",
            "NOTICE",
            "UNLICENSE",
            "COPYRIGHT",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix));
        if notice && entry.path().is_file() {
            found.insert(entry.path());
        }
    }
    Ok(found.into_iter().collect())
}

/// The generated folder's contents: relative path to bytes.
fn generate(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    let rust = root.join("rust");
    let mut crates = BTreeMap::new();
    for (target, _, _) in crate::archive::TARGETS {
        crates_for(&rust, target, &mut crates)?;
    }
    let mut files = BTreeMap::new();
    let mut index = String::from(
        "# Third-party licenses\n\n\
         Generated by `cargo xtask licenses`; don't edit. The shipped Russet\n\
         binaries link these crates on at least one release target. Each\n\
         folder holds the license and notice files the crate publishes.\n\n\
         | Crate | Version | License | Source |\n| --- | --- | --- | --- |\n",
    );
    let mut missing = Vec::new();
    for (key, krate) in &crates {
        let notices = notices(krate)?;
        if notices.is_empty() {
            missing.push(key.clone());
        }
        for notice in notices {
            let name = notice.file_name().ok_or("A notice has no name")?;
            let bytes = fs::read(&notice).map_err(|e| format!("{}: {e}", notice.display()))?;
            files.insert(PathBuf::from(key).join(name), bytes);
        }
        index.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            krate.name,
            krate.version,
            if krate.license.is_empty() {
                "See its folder"
            } else {
                &krate.license
            },
            if krate.vendored {
                "Vendored in `rust/vendor` with Russet patches"
            } else if krate.repository.is_empty() {
                "crates.io"
            } else {
                &krate.repository
            }
        ));
    }
    let mpl: Vec<&Crate> = crates
        .values()
        .filter(|c| c.license.contains("MPL-2.0"))
        .collect();
    if !mpl.is_empty() {
        index.push_str(
            "\n## Source for MPL-2.0 crates\n\n\
             These crates are used unmodified. Their source is available from\n\
             crates.io at the versions listed, and from their repositories:\n\n",
        );
        for krate in mpl {
            index.push_str(&format!(
                "- `{}` {}: <https://crates.io/crates/{}/{}>\n",
                krate.name, krate.version, krate.name, krate.version
            ));
        }
    }
    if !missing.is_empty() {
        index.push_str(
            "\n## Crates without license files\n\n\
             These crates publish no license file; their license is the one\n\
             in the table above.\n\n",
        );
        for key in missing {
            index.push_str(&format!("- `{key}`\n"));
        }
    }
    files.insert(PathBuf::from("README.md"), index.into_bytes());
    Ok(files)
}

fn existing(folder: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<(), String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Ok(());
        };
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(base, &path, out)?;
            } else {
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                out.insert(path.strip_prefix(base).unwrap().to_path_buf(), bytes);
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(folder, folder, &mut out)?;
    Ok(out)
}

/// Writes `rust/licenses`, or with `check`, fails when it differs from what
/// would be written.
pub fn licenses(root: &Path, check: bool) -> Result<(), String> {
    let folder = root.join("rust/licenses");
    let generated = generate(root)?;
    if check {
        let current = existing(&folder)?;
        if current != generated {
            let stale: Vec<String> = generated
                .keys()
                .chain(current.keys())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|path| generated.get(*path) != current.get(*path))
                .take(10)
                .map(|path| path.display().to_string())
                .collect();
            return Err(format!(
                "rust/licenses is out of date ({}); run `cargo xtask licenses`",
                stale.join(", ")
            ));
        }
        return Ok(());
    }
    if folder.exists() {
        fs::remove_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    }
    for (path, bytes) in &generated {
        let path = folder.join(path);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    println!("Wrote {} files to {}", generated.len(), folder.display());
    Ok(())
}

/// The license files to ship, as archive paths and bytes.
pub fn shipped(root: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    Ok(existing(&root.join("rust/licenses"))?
        .into_iter()
        .map(|(path, bytes)| {
            let name = path
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            (format!("licenses/third-party/{name}"), bytes)
        })
        .collect())
}
