//! `AutoPkgSourceFinder`: find the AutoPkg source folder that GitHub's
//! archive extracts to. A native port of the autopkg/recipes processor
//! (Apache-2.0).
//!
//! Inputs and outputs: run `russet processor-info AutoPkgSourceFinder`, or see
//! `AutoPkgSourceFinder` in `compatibility/community-processors.json`.
use crate::community_builders::{output, string};
use plist::Dictionary;
use std::fs;

pub(crate) fn execute(env: &mut Dictionary) -> Result<(), String> {
    let root = string(env, "input_path")?;
    // Python glob preserves directory enumeration order, and returns root/ on no match.
    let found = fs::read_dir(root)
        .ok()
        .and_then(|entries| {
            entries.filter_map(Result::ok).find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("autopkg-autopkg-")
            })
        })
        .map(|entry| entry.path());
    let path = found
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{}/", root.trim_end_matches('/')));
    output(format!("Found {path}"));
    env.insert("autopkg_path".into(), path.into());
    Ok(())
}
