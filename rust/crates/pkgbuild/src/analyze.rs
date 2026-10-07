//! `pkgbuild --analyze`: the bundles in a root folder.

use std::fs;
use std::io;
use std::path::Path;

/// Extensions of folders `pkgbuild` treats as bundles.
const BUNDLE_EXTENSIONS: [&str; 14] = [
    "app",
    "bundle",
    "framework",
    "plugin",
    "kext",
    "appex",
    "xpc",
    "prefPane",
    "saver",
    "qlgenerator",
    "mdimporter",
    "component",
    "docktileplugin",
    "systemextension",
];

/// One bundle in the component property list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Component {
    /// Path relative to the root.
    pub path: String,
    pub identifier: String,
    pub short_version: Option<String>,
    pub version: Option<String>,
    /// `BundleOverwriteAction`: `upgrade` for top-level bundles, empty for
    /// nested ones.
    pub overwrite_action: String,
    pub version_checked: bool,
    pub strict_identifier: bool,
    pub relocatable: bool,
    /// Nested bundles, for a top-level bundle.
    pub children: Vec<Component>,
}

fn info(bundle: &Path) -> Option<plist::Dictionary> {
    [
        "Contents/Info.plist",
        "Resources/Info.plist",
        "Versions/Current/Resources/Info.plist",
        "Info.plist",
    ]
    .iter()
    .map(|p| bundle.join(p))
    .find(|p| p.is_file())
    .and_then(|p| plist::from_file(p).ok())
}

fn component(root: &Path, path: &Path, top: bool) -> Option<Component> {
    let info = info(path)?;
    let text = |key: &str| {
        info.get(key)
            .and_then(plist::Value::as_string)
            .map(str::to_owned)
    };
    let identifier = text("CFBundleIdentifier")?;
    let app = path.extension().is_some_and(|e| e == "app");
    Some(Component {
        path: path.strip_prefix(root).ok()?.to_str()?.to_owned(),
        identifier,
        short_version: text("CFBundleShortVersionString"),
        version: text("CFBundleVersion"),
        overwrite_action: if top { "upgrade".into() } else { String::new() },
        version_checked: top,
        strict_identifier: app,
        relocatable: false,
        children: Vec::new(),
    })
}

fn is_bundle(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| BUNDLE_EXTENSIONS.contains(&e))
}

/// Walks `dir`, collecting bundles. Bundles inside a bundle are flattened
/// into its children.
fn walk(
    root: &Path,
    dir: &Path,
    top: Option<&mut Component>,
    out: &mut Vec<Component>,
    depth: usize,
) -> io::Result<()> {
    if depth > 64 {
        return Ok(());
    }
    let mut children: Vec<_> = fs::read_dir(dir)?.collect::<io::Result<_>>()?;
    children.sort_by_key(|e| e.file_name());
    let mut top = top;
    for entry in children {
        let path = entry.path();
        if !fs::symlink_metadata(&path)?.is_dir() {
            continue;
        }
        if is_bundle(&path) {
            if let Some(parent) = top.as_deref_mut() {
                if let Some(child) = component(root, &path, false) {
                    parent.children.push(child);
                }
                walk(root, &path, Some(parent), out, depth + 1)?;
            } else if let Some(mut bundle) = component(root, &path, true) {
                walk(root, &path, Some(&mut bundle), out, depth + 1)?;
                out.push(bundle);
            } else {
                walk(root, &path, None, out, depth + 1)?;
            }
        } else {
            walk(root, &path, top.as_deref_mut(), out, depth + 1)?;
        }
    }
    Ok(())
}

/// Finds the bundles under `root`, as `pkgbuild --analyze` does, with
/// `BundleIsRelocatable` off (as Russet's packaging helper sets it).
pub fn analyze(root: &Path) -> io::Result<Vec<Component>> {
    let mut out = Vec::new();
    walk(root, root, None, &mut out, 0)?;
    Ok(out)
}
