//! `StopProcessingIf`: evaluate a predicate against the environment and stop
//! the recipe when it's true.
use super::Output;
use crate::{string, Result};
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    #[cfg(target_os = "macos")]
    let result = autopkg_platform::predicate(string(env, "predicate")?, env)?;
    #[cfg(not(target_os = "macos"))]
    let result = crate::predicate::evaluate(string(env, "predicate")?, env)?;
    output(
        env,
        1,
        format!(
            "({}) is {}",
            string(env, "predicate")?,
            if result { "True" } else { "False" }
        ),
    );
    env.insert("stop_processing_recipe".into(), result.into());
    Ok(())
}
