//! `Versioner`: read a version from a property list, including one inside a
//! zip archive.
use super::Output;
use crate::{portable_path, read_dict, string, truth, Result};
use plist::{Dictionary, Value};
use std::path::Path;

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let path = string(env, "input_plist_path")?;
    let data = if path.to_lowercase().contains(".zip/") || path.to_lowercase().contains(".zip\\") {
        super::unarchiver::zip_plist(path, truth(env.get("skip_single_root_dir")))?
            .ok_or_else(|| format!("File '{path}' was not found."))?
    } else {
        portable_path(path)?;
        read_dict(Path::new(path))?
    };
    let key = env
        .get("plist_version_key")
        .and_then(Value::as_string)
        .unwrap_or("CFBundleShortVersionString");
    env.insert(
        "version".into(),
        data.get(key)
            .cloned()
            .unwrap_or_else(|| "UNKNOWN_VERSION".into()),
    );
    output(
        env,
        1,
        format!(
            "Found version {} in file {}",
            plist::python_str(&env["version"]),
            string(env, "input_plist_path")?
        ),
    );
    Ok(())
}
