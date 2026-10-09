//! `FindAndReplace`: replace text in a string and save the result to a
//! variable, `output_string` unless `result_output_var_name` names another.
//!
//! Inputs and outputs: run `russet processor-info FindAndReplace`, or see
//! `FindAndReplace` in `compatibility/reference.json`.
use super::Output;
use crate::{string, Result};
use plist::{Dictionary, Value};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let result =
        string(env, "input_string")?.replace(string(env, "find")?, string(env, "replace")?);
    let key = output_name(env);
    output(
        env,
        1,
        format!(
            "Replacing \"{}\" with \"{}\" in \"{}\" and saving result to \"{key}\" variable.",
            string(env, "find")?,
            string(env, "replace")?,
            string(env, "input_string")?
        ),
    );
    env.insert(key, result.into());
    Ok(())
}

/// The variable the result goes in. The dispatcher reads it before running,
/// because the processor may overwrite `result_output_var_name` itself.
pub(crate) fn output_name(env: &Dictionary) -> String {
    env.get("result_output_var_name")
        .and_then(Value::as_string)
        .unwrap_or("output_string")
        .to_owned()
}
