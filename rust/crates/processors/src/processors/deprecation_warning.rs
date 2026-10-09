//! `DeprecationWarning`: warn that a recipe is deprecated and add it to the
//! run's deprecation summary. Other deprecated processors use [`warn`] too.
//!
//! Inputs and outputs: run `russet processor-info DeprecationWarning`, or see
//! `DeprecationWarning` in `compatibility/reference.json`.
use crate::{string, Result};
use plist::{Dictionary, Value};
use std::path::Path;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let message = env
        .get("warning_message")
        .and_then(Value::as_string)
        .unwrap_or("### This recipe has been deprecated. It may be removed soon. ###")
        .to_string();
    warn(env, message)
}

/// Print `message` as a warning and record it in `deprecation_summary_result`
/// under the recipe's name.
pub(crate) fn warn(env: &mut Dictionary, message: String) -> Result<()> {
    let recipe = Path::new(string(env, "RECIPE_PATH")?)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let name = [".recipe.yaml", ".recipe.plist", ".recipe"]
        .iter()
        .find_map(|extension| recipe.strip_suffix(extension))
        .unwrap_or(&recipe)
        .to_string();
    let mut data = Dictionary::new();
    data.insert("name".into(), name.into());
    data.insert("warning".into(), message.clone().into());
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        "The following recipes have deprecation warnings:".into(),
    );
    summary.insert(
        "report_fields".into(),
        Value::Array(vec!["name".into(), "warning".into()]),
    );
    summary.insert("data".into(), data.into());
    env.insert("deprecation_summary_result".into(), summary.into());
    autopkg_platform::processor_output(1, format!("WARNING: {message}"));
    Ok(())
}
