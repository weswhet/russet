//! Verifies a signed bundle: its executable, its resource seal
//! (`_CodeSignature/CodeResources`), and its nested code, the way
//! `codesign --verify [--deep] [--strict]` does.

use crate::code::{self, CodeSignature, Sealed};
use crate::requirement::Requirement;
use sha2::Digest;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How thoroughly to verify.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Verify nested code completely, not just its hash and requirement.
    pub deep: bool,
    /// Reject symlinks that leave the bundle.
    pub strict: bool,
}

const MAX_DEPTH: usize = 16;
const MAX_FILES: usize = 500_000;
/// Extensions of bundles that can be nested code.
const BUNDLE_EXTENSIONS: [&str; 12] = [
    "app",
    "framework",
    "xpc",
    "appex",
    "bundle",
    "plugin",
    "kext",
    "docktileplugin",
    "qlgenerator",
    "mdimporter",
    "systemextension",
    "dext",
];

fn failure(path: &Path, message: impl std::fmt::Display) -> String {
    format!("{}: {message}", path.display())
}

/// The parts of a bundle that its signature covers.
struct Layout {
    /// The folder `CodeResources` paths are relative to.
    root: PathBuf,
    info: PathBuf,
    /// The main executable, or `None` for a bundle without one.
    executable: Option<PathBuf>,
    /// Whether `codesign` stored the signature as separate files in
    /// `_CodeSignature`, as it does for a main executable that isn't a
    /// Mach-O file, such as a shell script.
    detached: bool,
}

/// Verifies a bundle's main code signature: its executable's, or for a
/// bundle without one, the separate files in `_CodeSignature`.
fn main_signature(
    bundle: &Path,
    layout: &Layout,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    match &layout.executable {
        Some(executable) => {
            let bytes = fs::read(executable).map_err(|e| failure(executable, e))?;
            let signature = if layout.detached {
                code::verify_detached_code(&bytes, &layout.root.join("_CodeSignature"), sealed, now)
            } else {
                code::verify_binary(&bytes, sealed, now)
            };
            signature.map_err(|e| failure(bundle, e))
        }
        None => code::verify_detached(&layout.root.join("_CodeSignature"), sealed, now)
            .map_err(|e| failure(bundle, e)),
    }
}

fn info_dictionary(path: &Path) -> Result<plist::Dictionary, String> {
    plist::from_file(path).map_err(|e| failure(path, format!("can't read Info.plist: {e}")))
}

fn layout(bundle: &Path) -> Result<Layout, String> {
    let (root, info) = if bundle.join("Contents/Info.plist").is_file() {
        (bundle.join("Contents"), bundle.join("Contents/Info.plist"))
    } else if bundle.join("Versions/Current").exists() {
        let root =
            fs::canonicalize(bundle.join("Versions/Current")).map_err(|e| failure(bundle, e))?;
        if !root.starts_with(fs::canonicalize(bundle).map_err(|e| failure(bundle, e))?) {
            return Err(failure(
                bundle,
                "Versions/Current points outside the bundle",
            ));
        }
        let info = root.join("Resources/Info.plist");
        (root, info)
    } else if bundle.join("Info.plist").is_file() {
        (bundle.to_path_buf(), bundle.join("Info.plist"))
    } else {
        // A flat framework, such as one that ships in Firefox, keeps its
        // Info.plist in Resources, where codesign looks for it.
        (bundle.to_path_buf(), bundle.join("Resources/Info.plist"))
    };
    // Without CFBundleExecutable, the executable is named after the bundle.
    let declared = info_dictionary(&info)?
        .get("CFBundleExecutable")
        .and_then(plist::Value::as_string)
        .map(str::to_owned);
    let executable = declared
        .clone()
        .or_else(|| {
            bundle
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| failure(bundle, "bundle has no executable"))?;
    if executable.contains('/') {
        return Err(failure(bundle, "CFBundleExecutable isn't a file name"));
    }
    // A CodeDirectory file in _CodeSignature marks a detached signature.
    let detached = root.join("_CodeSignature/CodeDirectory").is_file();
    // A detached signature covers a main executable only when the bundle
    // declares one. A file that only shares the bundle's name, such as in
    // Adium's framework, isn't covered by it.
    let executable = if detached && declared.is_none() {
        None
    } else {
        [root.join("MacOS").join(&executable), root.join(&executable)]
            .into_iter()
            .find(|p| p.is_file())
    };
    Ok(Layout {
        root,
        info,
        executable,
        detached,
    })
}

/// One `rules2` entry.
struct Rule {
    pattern: regex::Regex,
    weight: f64,
    omit: bool,
    nested: bool,
}

fn rules(resources: &plist::Dictionary) -> Result<Vec<Rule>, String> {
    let rules = resources
        .get("rules2")
        .and_then(plist::Value::as_dictionary)
        .ok_or(
            "The resource seal has no rules2; signatures from before macOS 10.9 aren't supported",
        )?;
    rules
        .iter()
        .map(|(pattern, value)| {
            let options = value.as_dictionary();
            let flag = |key: &str| {
                options
                    .and_then(|o| o.get(key))
                    .and_then(plist::Value::as_boolean)
                    .unwrap_or(false)
            };
            let weight = options
                .and_then(|o| o.get("weight"))
                .and_then(|w| {
                    w.as_real()
                        .or_else(|| w.as_signed_integer().map(|i| i as f64))
                })
                .unwrap_or(1.0);
            Ok(Rule {
                pattern: regex::Regex::new(pattern)
                    .map_err(|e| format!("Bad resource rule {pattern:?}: {e}"))?,
                weight,
                omit: flag("omit"),
                nested: flag("nested"),
            })
        })
        .collect()
}

fn best_rule<'a>(rules: &'a [Rule], path: &str) -> Option<&'a Rule> {
    rules.iter().filter(|r| r.pattern.is_match(path)).fold(
        None,
        |best: Option<&Rule>, r| match best {
            Some(b) if b.weight >= r.weight => Some(b),
            _ => Some(r),
        },
    )
}

fn is_macho(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && matches!(
            u32::from_be_bytes(magic),
            0xcafe_babe | 0xcafe_babf | 0xcffa_edfe | 0xcefa_edfe | 0xfeed_face | 0xfeed_facf
        )
}

fn is_bundle(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| BUNDLE_EXTENSIONS.contains(&e))
}

struct Seal<'a> {
    bundle: &'a Path,
    root: &'a Path,
    executable: Option<&'a Path>,
    /// The bundle's own Info.plist, which the code directory seals.
    info: &'a Path,
    rules: Vec<Rule>,
    files: &'a plist::Dictionary,
    seen: Vec<String>,
    options: Options,
    now: SystemTime,
    depth: usize,
    count: usize,
    /// The outermost bundle being verified. With `--strict`, symlinks may
    /// leave a nested bundle but not this one: helper apps in Qt and
    /// Electron apps link to their host's Frameworks folder.
    scope: &'a Path,
}

impl Seal<'_> {
    /// Whether the seal records `key` by code directory hash.
    fn sealed_as_code(&self, key: &str) -> bool {
        self.files
            .get(key)
            .and_then(plist::Value::as_dictionary)
            .is_some_and(|e| e.contains_key("cdhash"))
    }

    /// The seal's entry for `key`. Older seals record a plain file as its
    /// SHA-1 hash alone, which is read as `{hash: ...}`.
    fn entry(&self, key: &str) -> Result<plist::Dictionary, String> {
        self.files
            .get(key)
            .and_then(|value| match value {
                plist::Value::Dictionary(entry) => Some(entry.clone()),
                plist::Value::Data(hash) => Some(plist::Dictionary::from_iter([(
                    "hash".to_owned(),
                    plist::Value::Data(hash.clone()),
                )])),
                _ => None,
            })
            .ok_or_else(|| {
                failure(
                    self.bundle,
                    format!("a sealed resource is missing or invalid: file added: {key}"),
                )
            })
    }

    fn walk(&mut self, dir: &Path) -> Result<(), String> {
        let mut children: Vec<_> = fs::read_dir(dir)
            .map_err(|e| failure(dir, e))?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()
            .map_err(|e| failure(dir, e))?;
        children.sort();
        for path in children {
            self.count += 1;
            if self.count > MAX_FILES {
                return Err(failure(self.bundle, "too many files to verify"));
            }
            let key = path
                .strip_prefix(self.root)
                .unwrap()
                .to_str()
                .ok_or_else(|| failure(&path, "name isn't UTF-8"))?
                .to_owned();
            // The signature itself and the legacy top-level link to it
            // aren't resources. The main executable and the Info.plist are
            // sealed by the code directory, and are resources only when an
            // older signature lists them.
            let own = Some(path.as_path()) == self.executable || path == self.info;
            if key == "_CodeSignature"
                || key == "CodeResources"
                || (own && !self.files.contains_key(&key))
            {
                continue;
            }
            let Some(rule) = best_rule(&self.rules, &key) else {
                return Err(failure(
                    self.bundle,
                    format!("no resource rule covers {key}"),
                ));
            };
            if rule.omit {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(|e| failure(&path, e))?;
            if metadata.file_type().is_symlink() {
                self.symlink(&path, &key)?;
            } else if metadata.is_dir() {
                // A folder is nested code when the seal records it by code
                // directory hash, whatever its extension.
                if rule.nested && (self.sealed_as_code(&key) || is_bundle(&path)) {
                    self.nested(&path, &key)?;
                } else {
                    self.walk(&path)?;
                }
            } else if rule.nested && (is_macho(&path) || self.sealed_as_code(&key)) {
                // Non-Mach-O files in nested-code locations are signed as
                // code, with the signature in com.apple.cs.* attributes.
                self.check_sideband(&path)?;
                self.nested(&path, &key)?;
            } else {
                self.check_sideband(&path)?;
                let entry = self.entry(&key)?;
                let ok = match (entry.get("hash2"), entry.get("hash")) {
                    (Some(expected), _) => {
                        let digest = digest_file::<sha2::Sha256>(&path)?;
                        expected.as_data() == Some(digest.as_slice())
                    }
                    (None, Some(expected)) => {
                        let digest = digest_file::<sha1::Sha1>(&path)?;
                        expected.as_data() == Some(digest.as_slice())
                    }
                    _ => false,
                };
                if !ok {
                    return Err(failure(
                        self.bundle,
                        format!("a sealed resource is missing or invalid: file modified: {key}"),
                    ));
                }
                self.seen.push(key);
            }
        }
        Ok(())
    }

    /// With `--strict`, a file the seal covers can't have a resource fork or
    /// Finder info. Files the seal omits, such as `.DS_Store`, may.
    fn check_sideband(&self, path: &Path) -> Result<(), String> {
        if self.options.strict {
            check_detritus(self.bundle, path)?;
        }
        Ok(())
    }

    fn symlink(&mut self, path: &Path, key: &str) -> Result<(), String> {
        let target = fs::read_link(path).map_err(|e| failure(path, e))?;
        if self.options.strict {
            let resolved = path.parent().unwrap().join(&target);
            let normalized = normalize(&resolved);
            if target.is_absolute() || !normalized.starts_with(normalize(self.scope)) {
                return Err(failure(
                    self.bundle,
                    format!("invalid symlink {key} points outside the bundle"),
                ));
            }
        }
        let entry = self.entry(key)?;
        if entry.get("symlink").and_then(plist::Value::as_string) != target.to_str() {
            return Err(failure(
                self.bundle,
                format!("a sealed resource is missing or invalid: symlink modified: {key}"),
            ));
        }
        self.seen.push(key.to_owned());
        Ok(())
    }

    fn nested(&mut self, path: &Path, key: &str) -> Result<(), String> {
        let entry = self.entry(key)?;
        let cdhash = entry
            .get("cdhash")
            .and_then(plist::Value::as_data)
            .ok_or_else(|| {
                failure(
                    self.bundle,
                    format!("nested code {key} isn't sealed by hash"),
                )
            })?
            .to_vec();
        let requirement = entry
            .get("requirement")
            .and_then(plist::Value::as_string)
            .map(Requirement::parse)
            .transpose()?;
        let signature = if self.options.deep || !path.is_dir() {
            if path.is_dir() {
                verify_at_depth(path, self.scope, self.options, self.now, self.depth + 1)?
            } else {
                verify_file(path, self.now)?
            }
        } else {
            // Without --deep, only the nested executable's signature is
            // checked, which is what its hash and requirement cover.
            let inner = layout(path)?;
            let info = fs::read(&inner.info).map_err(|e| failure(&inner.info, e))?;
            let resources = fs::read(inner.root.join("_CodeSignature/CodeResources")).ok();
            main_signature(
                path,
                &inner,
                &Sealed {
                    info_plist: Some(&info),
                    resources: resources.as_deref(),
                    bundle: true,
                },
                self.now,
            )?
        };
        // When the seal records a requirement, codesign accepts nested code
        // that was re-signed after the seal was made, as long as its
        // signature is valid and satisfies that requirement; the recorded
        // hash is then informational (Raycast's and iClicker's apps ship
        // like this). Without a requirement, the hash must match.
        match requirement {
            Some(requirement) => {
                if !signature.satisfies(&requirement) {
                    return Err(failure(
                        self.bundle,
                        format!("a sealed resource is missing or invalid: nested code modified: {key} doesn't satisfy its sealed requirement"),
                    ));
                }
            }
            None if !signature.cdhashes.contains(&cdhash) => {
                return Err(failure(
                    self.bundle,
                    format!("a sealed resource is missing or invalid: nested code modified: {key}"),
                ));
            }
            None => {}
        }
        self.seen.push(key.to_owned());
        Ok(())
    }
}

/// Attributes `codesign --strict` calls detritus, by Apple name; Russet's
/// extractors may store them in the `user.` namespace or the sidecar.
#[cfg(unix)]
const DETRITUS: [&str; 2] = ["com.apple.FinderInfo", "com.apple.ResourceFork"];

/// Fails when `path`, the bundle itself or one of the files its signature
/// covers, has a resource fork or Finder info, which `codesign --strict`
/// rejects. Folders inside the bundle, such as document packages, and files
/// the seal omits may have them.
#[cfg(unix)]
fn check_detritus(bundle: &Path, path: &Path) -> Result<(), String> {
    for name in DETRITUS {
        if russet_fs::get_xattr(path, name).ok().flatten().is_some() {
            let shown = path.strip_prefix(bundle).unwrap_or(path);
            return Err(failure(
                bundle,
                format!(
                    "resource fork, Finder information, or similar detritus not allowed: {name} on {}",
                    if shown.as_os_str().is_empty() {
                        Path::new(".")
                    } else {
                        shown
                    }
                    .display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_detritus(_bundle: &Path, _path: &Path) -> Result<(), String> {
    Ok(())
}

/// Removes `.` and `..` lexically, without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            // A leading `..` in a relative path must stay, or the path would
            // compare equal to one that is inside the bundle.
            std::path::Component::ParentDir => match out.components().next_back() {
                Some(std::path::Component::Normal(_)) => {
                    out.pop();
                }
                Some(std::path::Component::RootDir) => {}
                _ => out.push(".."),
            },
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Hashes a file in fixed-size reads, so a multi-gigabyte resource doesn't
/// have to fit in memory.
fn digest_file<D: Digest + std::io::Write>(path: &Path) -> Result<Vec<u8>, String> {
    let mut file = fs::File::open(path).map_err(|e| failure(path, e))?;
    let mut hasher = D::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| failure(path, e))?;
    Ok(hasher.finalize().to_vec())
}

/// Verifies a bundle or a single Mach-O file at time `now`.
pub fn verify(path: &Path, options: Options, now: SystemTime) -> Result<CodeSignature, String> {
    // Paths below the bundle are absolute, so the symlink check must compare
    // against an absolute bundle path too.
    let path = fs::canonicalize(path).map_err(|e| failure(path, e))?;
    verify_at_depth(&path, &path, options, now, 0)
}

/// Verifies a single file's signature: a Mach-O binary's embedded one, or
/// for any other file, the one `codesign` stores in its `com.apple.cs.*`
/// extended attributes.
fn verify_file(path: &Path, now: SystemTime) -> Result<CodeSignature, String> {
    let bytes = fs::read(path).map_err(|e| failure(path, e))?;
    if is_macho(path) {
        return code::verify_binary(&bytes, &Sealed::default(), now).map_err(|e| failure(path, e));
    }
    #[cfg(unix)]
    {
        let read = |name: &str| {
            russet_fs::get_xattr(path, &format!("com.apple.cs.{name}"))
                .ok()
                .flatten()
        };
        code::verify_components_from(&bytes, read, now).map_err(|e| failure(path, e))
    }
    #[cfg(not(unix))]
    Err(failure(path, "code object is not signed at all"))
}

fn verify_at_depth(
    path: &Path,
    scope: &Path,
    options: Options,
    now: SystemTime,
    depth: usize,
) -> Result<CodeSignature, String> {
    if depth > MAX_DEPTH {
        return Err(failure(path, "nested code is too deep"));
    }
    if !path.is_dir() {
        return verify_file(path, now);
    }
    if options.strict && depth == 0 {
        check_detritus(path, path)?;
    }
    let layout = layout(path)?;
    if options.strict {
        // The code directory seals the executable rather than
        // CodeResources, so the resource walk below skips it. codesign
        // doesn't check the Info.plist.
        if let Some(executable) = &layout.executable {
            check_detritus(path, executable)?;
        }
    }
    let info = fs::read(&layout.info).map_err(|e| failure(&layout.info, e))?;
    let resources_path = layout.root.join("_CodeSignature/CodeResources");
    let resources = fs::read(&resources_path).ok();
    let signature = main_signature(
        path,
        &layout,
        &Sealed {
            info_plist: Some(&info),
            resources: resources.as_deref(),
            bundle: true,
        },
        now,
    )?;
    let Some(resources) = resources else {
        return Err(failure(
            path,
            "code has no resources but signature indicates they must be present",
        ));
    };
    let seal: plist::Dictionary =
        plist::from_bytes(&resources).map_err(|e| failure(&resources_path, e))?;
    let files = seal
        .get("files2")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| failure(&resources_path, "no files2"))?;
    let mut walker = Seal {
        bundle: path,
        root: &layout.root,
        executable: layout.executable.as_deref(),
        info: &layout.info,
        rules: rules(&seal)?,
        files,
        seen: Vec::new(),
        options,
        now,
        depth,
        count: 0,
        scope,
    };
    walker.walk(&layout.root)?;
    let seen: std::collections::HashSet<&str> = walker.seen.iter().map(String::as_str).collect();
    let missing: BTreeMap<&String, &plist::Value> = files
        .iter()
        .filter(|(key, value)| {
            let optional = value
                .as_dictionary()
                .and_then(|d| d.get("optional"))
                .and_then(plist::Value::as_boolean)
                .unwrap_or(false);
            !seen.contains(key.as_str()) && !optional
        })
        .collect();
    if let Some((key, _)) = missing.into_iter().next() {
        return Err(failure(
            path,
            format!("a sealed resource is missing or invalid: file missing: {key}"),
        ));
    }
    Ok(signature)
}
