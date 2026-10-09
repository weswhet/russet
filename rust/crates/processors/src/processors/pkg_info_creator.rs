//! `PkgInfoCreator`: write a flat package's PackageInfo file from a template,
//! with the payload's size and file count.
//!
//! Inputs and outputs: run `russet processor-info PkgInfoCreator`, or see
//! `PkgInfoCreator` in `compatibility/reference.json`.
use crate::{io, read_dict, string, Result};
use autopkg_platform::processor_output as output;
use plist::Dictionary;
use std::{
    fs,
    path::{Path, PathBuf},
};
use xmltree::{Element, XMLNode};

fn find_template(env: &Dictionary) -> Result<PathBuf> {
    let path = Path::new(string(env, "template_path")?);
    if path.exists() {
        return Ok(path.to_path_buf());
    }
    if path.is_relative() {
        let mut directories = Vec::new();
        if let Some(directory) = env.get("RECIPE_DIR").and_then(plist::Value::as_string) {
            directories.push(PathBuf::from(directory));
        }
        if let Some(parents) = env.get("PARENT_RECIPES").and_then(plist::Value::as_array) {
            for parent in parents {
                let path = Path::new(
                    parent
                        .as_string()
                        .ok_or("PARENT_RECIPES must contain strings")?,
                );
                if let Some(dir) = path.parent() {
                    directories.push(dir.to_path_buf());
                }
            }
        }
        for directory in directories {
            let candidate = directory.join(path);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Err(format!("Can't find {}", path.display()))
}
fn template(path: &Path) -> Result<Element> {
    if path.extension().is_some_and(|e| e == "plist") {
        let data = read_dict(path)?;
        let mut element = Element::new("pkg-info");
        element
            .attributes
            .insert("format-version".into(), "2".into());
        for (source, target) in [
            ("IFPkgFlagDefaultLocation", "install-location"),
            ("CFBundleShortVersionString", "version"),
            ("CFBundleIdentifier", "identifier"),
        ] {
            if let Some(value) = data.get(source) {
                element.attributes.insert(
                    target.into(),
                    value
                        .as_string()
                        .ok_or_else(|| format!("{source} must be a string"))?
                        .into(),
                );
            }
        }
        if let Some(auth) = data.get("IFPkgFlagAuthorizationAction") {
            element.attributes.insert(
                "auth".into(),
                if auth.as_string() == Some("RootAuthorization") {
                    "root"
                } else {
                    "none"
                }
                .into(),
            );
        }
        if let Some(action) = data.get("IFPkgFlagRestartAction") {
            let action = match action.as_string() {
                Some("RecommendRestart" | "RequireRestart") => "restart",
                Some("RequireLogout") => "logout",
                Some("RequireShutdown") => "shutdown",
                Some("None") => "none",
                _ => {
                    output(1, format!("WARNING: Unrecognized IFPkgFlagRestartAction '{}' in template; treating as 'none'.", plist::python_str(action)));
                    "none"
                }
            };
            element
                .attributes
                .insert("postinstall-action".into(), action.into());
        }
        element
            .children
            .push(XMLNode::Element(Element::new("payload")));
        Ok(element)
    } else {
        Element::parse(io(fs::File::open(path))?)
            .map_err(|e| format!("Malformed PackageInfo template {}: {e}", path.display()))
    }
}
fn size(root: &Path) -> Result<(u64, u64)> {
    // Match os.walk: nonexistent roots produce zero counts and directory symlinks
    // are listed as directories but never visited or included in payload size.
    if !root.is_dir() {
        return Ok((0, 0));
    }
    let mut blocks = 0;
    let mut files = 1;
    for entry in io(fs::read_dir(root))? {
        let path = io(entry)?.path();
        let metadata = io(fs::symlink_metadata(&path))?;
        if path.is_dir() {
            if !metadata.file_type().is_symlink() {
                let (b, f) = size(&path)?;
                blocks += b;
                files += f;
            }
        } else {
            blocks += metadata.len().div_ceil(4096);
            files += 1;
        }
    }
    Ok((blocks, files))
}
pub(crate) fn execute(env: &Dictionary) -> Result<()> {
    let kind = string(env, "pkgtype")?;
    if kind != "flat" && kind != "bundle" {
        return Err(format!("Unknown pkgtype {kind}"));
    }
    let mut info = template(&find_template(env)?)?;
    if kind == "bundle" {
        return Err("Bundle package creation no longer supported!".into());
    }
    if info.name != "pkg-info" || info.namespace.is_some() {
        return Err("PackageInfo root should be pkg-info".into());
    }
    info.attributes
        .insert("version".into(), string(env, "version")?.into());
    if info.get_child("payload").is_none() {
        info.children
            .push(XMLNode::Element(Element::new("payload")));
    }
    let payload = info.get_mut_child("payload").unwrap();
    let (blocks, files) = size(Path::new(string(env, "pkgroot")?))?;
    payload
        .attributes
        .insert("installKBytes".into(), blocks.to_string());
    payload
        .attributes
        .insert("numberOfFiles".into(), files.to_string());
    info.write_with_config(
        io(fs::File::create(string(env, "infofile")?))?,
        xmltree::EmitterConfig::new().write_document_declaration(false),
    )
    .map_err(|e| e.to_string())
}
