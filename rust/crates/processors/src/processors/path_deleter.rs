//! `PathDeleter`: delete files and folders, retrying folders that fail.
use super::Output;
use crate::{remove, truth, Result};
use plist::{Dictionary, Value};
use std::{fs, path::Path};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    if let Some(Value::String(path)) = env.get("path_list") {
        env.insert("path_list".into(), Value::Array(vec![path.clone().into()]));
    }
    let paths = env
        .get("path_list")
        .and_then(Value::as_array)
        .ok_or("path_list must be an array")?;
    for value in paths {
        let path = Path::new(
            value
                .as_string()
                .ok_or("path_list entries must be strings")?,
        );
        let existed = fs::symlink_metadata(path).is_ok();
        if !existed && !path.exists() {
            if truth(env.get("continue_on_error")) {
                output(
                    env,
                    1,
                    format!("Path does not exist, skipping: {}", path.display()),
                );
                continue;
            }
            return Err(format!("Could not remove {} - it does not exist! Set continue_on_error=True to skip missing paths.", path.display()));
        }
        let directory = path.is_dir() && !path.is_symlink();
        let mut result = remove(path);
        if directory {
            for (attempt, delay) in [(1, 1), (2, 2)] {
                if result.is_ok() {
                    break;
                }
                output(
                    env,
                    1,
                    format!(
                        "Unable to remove {} (attempt {attempt} of 3); retrying in {delay}s",
                        path.display()
                    ),
                );
                std::thread::sleep(std::time::Duration::from_secs(delay));
                result = remove(path);
            }
            if let Err(error) = &result {
                if truth(env.get("continue_on_error")) {
                    output(
                        env,
                        1,
                        format!("Ignoring errors on final removal of {}", path.display()),
                    );
                    let _ = remove(path);
                    continue;
                }
                return Err(format!(
                    "Could not remove {} after 3 attempts: {error}",
                    path.display()
                ));
            }
        }
        match &result {
            Ok(()) => output(env, 1, format!("Deleted {}", path.display())),
            Err(error) if truth(env.get("continue_on_error")) => output(
                env,
                1,
                format!("Ignoring error removing {}: {error}", path.display()),
            ),
            Err(_) => {}
        }
        if !truth(env.get("continue_on_error")) {
            result?;
        }
    }
    Ok(())
}
