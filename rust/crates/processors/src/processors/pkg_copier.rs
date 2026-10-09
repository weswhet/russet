//! `PkgCopier`: copy a package, found with a glob, to the recipe cache or
//! `pkg_path`.
use crate::package::log_glob;
use crate::{string, Result};
use autopkg_platform::processor_output as output;
use plist::Dictionary;
use std::path::{Path, PathBuf};

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    env.remove("pkg_copier_summary_result");
    let source = string(env, "source_pkg")?;
    crate::portable_path(source)?;
    let paths = crate::python_glob::paths_with_recursion(source, false)?;
    let source = paths
        .first()
        .ok_or("Error processing source_pkg with glob")?;
    log_glob("source_pkg", string(env, "source_pkg")?, &paths);
    if !source
        .extension()
        .is_some_and(|e| e == "pkg" || e == "mpkg")
    {
        return Err(format!(
            "Source does not appear to be a package based on its filename: '{}'",
            source.display()
        ));
    }
    let destination = match env
        .get("pkg_path")
        .and_then(plist::Value::as_string)
        .filter(|s| !s.is_empty())
    {
        Some(path) => PathBuf::from(path),
        None => Path::new(string(env, "RECIPE_CACHE_DIR")?).join(source.file_name().unwrap()),
    };
    // Keep Copier's temporary input keys out of the caller's environment.
    let mut inputs = Dictionary::new();
    inputs.insert(
        "source_path".into(),
        source.to_string_lossy().into_owned().into(),
    );
    inputs.insert(
        "destination_path".into(),
        destination.to_string_lossy().into_owned().into(),
    );
    inputs.insert("overwrite".into(), true.into());
    crate::execute("Copier", &mut inputs)?;
    output(
        1,
        format!("Copied {} to {}", source.display(), destination.display()),
    );
    let destination = destination.to_string_lossy().into_owned();
    env.insert("pkg_path".into(), destination.clone().into());
    let mut data = Dictionary::new();
    data.insert("pkg_path".into(), destination.into());
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        "The following packages were copied:".into(),
    );
    summary.insert("data".into(), data.into());
    env.insert("pkg_copier_summary_result".into(), summary.into());
    Ok(())
}
