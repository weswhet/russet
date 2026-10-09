//! `PlistEditor`: merge keys into a property list and write the result.
//!
//! Inputs and outputs: run `russet processor-info PlistEditor`, or see
//! `PlistEditor` in `compatibility/reference.json`.
use super::Output;
use crate::{read_dict, string, Result};
use plist::{Dictionary, Value};
use std::path::Path;

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let mut data = match env.get("input_plist_path").and_then(Value::as_string) {
        Some(p) if !p.is_empty() => read_dict(Path::new(p))?,
        _ => Dictionary::new(),
    };
    let edits = env
        .get("plist_data")
        .and_then(Value::as_dictionary)
        .ok_or("plist_data must be a dictionary")?;
    for (k, v) in edits {
        data.insert(k.clone(), v.clone());
    }
    Value::Dictionary(data)
        .to_file_xml(string(env, "output_plist_path")?)
        .map_err(|e| e.to_string())?;
    output(
        env,
        1,
        format!("Updated plist at {}", string(env, "output_plist_path")?),
    );
    Ok(())
}
