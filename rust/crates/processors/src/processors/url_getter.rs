//! `URLGetter`: in AutoPkg, the base class that runs curl for the processors
//! that download. It isn't a processor a recipe runs; running it directly
//! fails as it does in Python. These helpers fetch text with curl for
//! `URLTextSearcher` and the community processors.
use crate::{string, Result};
use plist::{Dictionary, Value};
use std::process::Command;

pub(crate) const ERROR: &str = "'URLGetter' object has no attribute 'input_variables'";

pub(crate) fn execute() -> Result<()> {
    Err(ERROR.into())
}

fn command(env: &Dictionary) -> Result<Command> {
    let binary = autopkg_platform::downloads::curl_binary(env)?;
    let mut command = Command::new(&binary);
    if !(cfg!(windows)
        && binary
            .to_string_lossy()
            .to_lowercase()
            .contains("windows\\system32"))
    {
        command.arg("--compressed");
    }
    command.arg("--location");
    if let Some(headers) = env.get("request_headers") {
        for (key, value) in headers
            .as_dictionary()
            .ok_or("request_headers must be a dictionary")?
        {
            let value = value
                .as_string()
                .ok_or("request_headers values must be strings")?;
            command.arg("--header").arg(format!("{key}: {value}"));
        }
    }
    if let Some(options) = env.get("curl_opts") {
        for option in options.as_array().ok_or("curl_opts must be an array")? {
            command.arg(
                option
                    .as_string()
                    .ok_or("curl_opts values must be strings")?,
            );
        }
    }
    command.arg(string(env, "url")?);
    Ok(command)
}
pub(crate) fn text(bytes: &[u8]) -> String {
    // Python's subprocess text mode uses errors="ignore" and universal newlines.
    let mut rest = bytes;
    let mut result = String::new();
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                result.push_str(s);
                break;
            }
            Err(e) => {
                result.push_str(std::str::from_utf8(&rest[..e.valid_up_to()]).unwrap());
                match e.error_len() {
                    Some(n) => rest = &rest[e.valid_up_to() + n..],
                    None => break,
                }
            }
        }
    }
    result.replace("\r\n", "\n").replace('\r', "\n")
}
pub(crate) fn fetch(env: &Dictionary) -> Result<String> {
    let mut command = command(env)?;
    let _certificate =
        super::url_downloader::trust::native_curl(&mut command).map_err(|e| e.message)?;
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|s| Value::String(s.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    let result = command
        .output()
        .map_err(|e| format!("Unable to execute curl: {e}"))?;
    if !result.status.success() {
        let error = text(&result.stderr);
        autopkg_platform::processor_output(
            1,
            format!("ERROR: {}", error.strip_prefix("curl: ").unwrap_or(&error)),
        );
        return Err(error);
    }
    autopkg_platform::processor_output(
        4,
        format!(
            "Curl command: {}",
            plist::python_repr(&Value::Array(arguments))
        ),
    );
    Ok(text(&result.stdout))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_utf8_is_ignored() {
        assert_eq!(text(b"a\xffb\r\nc\rd"), "ab\nc\nd");
    }
}
