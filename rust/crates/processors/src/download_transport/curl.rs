//! Runs a download's curl command and turns its output into response headers,
//! the effective URL, and failures. The command itself is built by the
//! downloader; this module only executes it.
use plist::Value;
use std::collections::BTreeMap;
use std::process::Command;

pub(crate) type Headers = BTreeMap<String, String>;
pub(crate) fn curl_stderr(bytes: &[u8]) -> String {
    // Python subprocess text=True applies universal newline decoding before
    // Processor.output writes through the platform's text stream. Keeping raw
    // CRLF here would turn it into CRCRLF when Windows output translates LF.
    crate::processors::url_getter::text(bytes)
}

pub(crate) fn parse_headers(text: &str) -> Headers {
    let mut headers = Headers::new();
    let mut redirected = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with("HTTP/") {
            headers.clear();
            if let Some(code) = line.split_whitespace().nth(1) {
                headers.insert("http_result_code".into(), code.into());
            }
            if let Some(reason) = line.splitn(3, ' ').nth(2) {
                headers.insert("http_result_description".into(), reason.into());
            }
        } else if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.to_lowercase(), value.trim().into());
        } else if line.is_empty()
            && headers
                .get("http_result_code")
                .is_some_and(|s| ["301", "302", "303", "307", "308"].contains(&s.as_str()))
        {
            redirected = headers.get("location").cloned();
        }
    }
    if let Some(url) = redirected {
        headers.insert("http_redirected".into(), url);
    }
    headers
}

#[derive(Debug)]
pub(crate) struct TransportFailure {
    pub(crate) failure: crate::ExecutionFailure,
    pub(crate) incomplete: Option<Headers>,
}
impl From<String> for TransportFailure {
    fn from(message: String) -> Self {
        Self {
            failure: message.into(),
            incomplete: None,
        }
    }
}
impl From<TransportFailure> for crate::ExecutionFailure {
    fn from(error: TransportFailure) -> Self {
        error.failure
    }
}
pub(crate) fn arguments(command: &Command) -> String {
    let arguments = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|s| Value::String(s.to_string_lossy().into_owned()))
        .collect::<Vec<_>>();
    plist::python_repr(&Value::Array(arguments))
}

/// Applies the downloader's handling of a curl exit status. `code` is curl's
/// exit code, `None` when curl did not exit normally. The native engine
/// reports its failures with the same codes, so both backends fail alike.
pub(crate) fn check_exit(
    python: bool,
    code: Option<i32>,
    response_headers: Headers,
    message: impl FnOnce() -> String,
) -> std::result::Result<(), TransportFailure> {
    let chunked = response_headers
        .get("transfer-encoding")
        .is_some_and(|s| s.to_ascii_lowercase().contains("chunked"));
    if python && code == Some(18) && chunked {
        return Err(TransportFailure {
            failure: crate::ExecutionFailure::unexpected(
                "IncompleteRead: incomplete chunked response",
            ),
            incomplete: Some(response_headers),
        });
    }
    // urllib's fixed-size read loop accepts EOF before Content-Length, but
    // incomplete chunk framing raises IncompleteRead. curl uses exit 18 for both.
    let accepted_short_body =
        python && code == Some(18) && !chunked && response_headers.contains_key("content-length");
    if code != Some(0) && !accepted_short_body {
        if python && code == Some(22) {
            let headers = &response_headers;
            if let Some(code) = headers.get("http_result_code") {
                return Err(TransportFailure {
                    failure: crate::ExecutionFailure::unexpected(format!(
                        "HTTP Error {code}: {}",
                        headers
                            .get("http_result_description")
                            .map(String::as_str)
                            .unwrap_or("")
                    )),
                    incomplete: None,
                });
            }
        }
        let message = message();
        if python {
            // urllib propagates transport exceptions (including TLS verification
            // failures) rather than wrapping them in AutoPkg's ProcessorError.
            return Err(TransportFailure {
                failure: crate::ExecutionFailure::unexpected(message),
                incomplete: None,
            });
        }
        autopkg_platform::processor_output(
            1,
            format!(
                "ERROR: {}",
                message.strip_prefix("curl: ").unwrap_or(&message)
            ),
        );
        return Err(message.into());
    }
    Ok(())
}

pub(crate) fn execute(
    mut command: Command,
    python: bool,
) -> std::result::Result<(Headers, String), TransportFailure> {
    let arguments = arguments(&command);
    let output = command
        .output()
        .map_err(|e| format!("Unable to execute curl: {e}"))?;
    check_exit(
        python,
        output.status.code(),
        parse_headers(&String::from_utf8_lossy(&output.stdout)),
        || curl_stderr(&output.stderr),
    )?;
    if !python {
        autopkg_platform::processor_output(4, format!("Curl command: {arguments}"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (headers, effective) = text
        .rsplit_once("\nAUTOPKG_EFFECTIVE_URL:")
        .unwrap_or((&text, ""));
    Ok((parse_headers(headers), effective.trim().into()))
}
