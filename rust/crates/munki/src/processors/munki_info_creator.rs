//! `MunkiInfoCreator`: generate a Munki pkginfo for a package, the way
//! `makepkginfo` does, and optionally write it to a file.
//!
//! Inputs and outputs: run `russet processor-info MunkiInfoCreator`, or see
//! `MunkiInfoCreator` in `compatibility/reference.json`.
use crate::{metadata, string};
use plist::{Dictionary, Value};
use std::path::Path;

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    let mut arguments = Vec::new();
    for key in ["displayname", "description", "catalog"] {
        if let Some(v) = env.get(key).and_then(Value::as_string) {
            arguments.push(format!("--{key}={v}"));
        }
    }
    let options = metadata::Options::parse(&arguments)?;
    let mut info = metadata::generate(Some(Path::new(string(env, "pkg_path")?)), &options)?;
    for key in ["name", "version"] {
        if let Some(v) = env.get(key) {
            info.insert(key.into(), v.clone());
        }
    }
    if let Some(path) = env.get("info_path").and_then(Value::as_string) {
        Value::Dictionary(info.clone())
            .to_file_xml(path)
            .map_err(|e| e.to_string())?;
    }
    env.insert("munki_info".into(), Value::Dictionary(info));
    Ok(())
}
