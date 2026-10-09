//! `FileFinder`: find the last file, in sorted order, that matches a glob.
use super::Output;
use crate::{matches, string, Result};
use plist::{Dictionary, Value};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let method = env
        .get("find_method")
        .and_then(Value::as_string)
        .unwrap_or("glob");
    if method != "glob" {
        return Err(format!("Unsupported find_method: {method}"));
    }
    let mut paths = matches(string(env, "pattern")?)?;
    paths.sort();
    let path = paths.last().ok_or("No matching filename found")?;
    env.insert(
        "found_filename".into(),
        path.to_string_lossy().into_owned().into(),
    );
    output(
        env,
        1,
        format!(
            "Found file match: '{}' from globbed '{}'",
            path.display(),
            string(env, "pattern")?
        ),
    );
    env.insert(
        "found_basename".into(),
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    output(
        env,
        1,
        format!("Basename match: '{}'", string(env, "found_basename")?),
    );
    Ok(())
}
