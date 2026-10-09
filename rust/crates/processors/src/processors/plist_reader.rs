//! `PlistReader`: copy keys from a property list, or a bundle's Info.plist,
//! into environment variables.
use super::Output;
use crate::{info_path, normalized_path, read_dict, string, Result};
use plist::{Dictionary, Value};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let normalized = normalized_path(string(env, "info_path")?);
    let p = info_path(&normalized.to_string_lossy())?;
    output(env, 1, format!("Reading: {}", p.display()));
    let data = read_dict(&p)?;
    let mut default = Dictionary::new();
    default.insert("CFBundleShortVersionString".into(), "version".into());
    let keys = env
        .get("plist_keys")
        .and_then(Value::as_dictionary)
        .unwrap_or(&default)
        .clone();
    env.insert(
        "plist_reader_output_variables".into(),
        Dictionary::new().into(),
    );
    for (key, target) in keys {
        let target = target
            .as_string()
            .ok_or("plist_keys values must be strings")?;
        let value = data
            .get(&key)
            .ok_or_else(|| {
                format!(
                    "Key '{key}' could not be found in the plist {}!",
                    p.display()
                )
            })?
            .clone();
        env.insert(target.into(), value.clone());
        output(
            env,
            1,
            format!(
                "Assigning value of '{}' to output variable '{target}'",
                plist::python_str(&value)
            ),
        );
        env.get_mut("plist_reader_output_variables")
            .unwrap()
            .as_dictionary_mut()
            .unwrap()
            .insert(target.into(), value);
    }
    Ok(())
}
