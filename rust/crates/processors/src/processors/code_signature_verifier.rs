//! `CodeSignatureVerifier`: verify the code signature of an app, a bundle, or
//! an installer package found with a glob. The platform crate does the
//! verification, natively where Russet supports it.
use crate::{python_glob, Result};
use plist::{Dictionary, Value};

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let matches = match env.get("input_path") {
        Some(Value::String(pattern)) => python_glob::paths_with_recursion(pattern, false)?,
        _ => Vec::new(),
    };
    autopkg_platform::signature::verify_code_signature(env, matches)
}
