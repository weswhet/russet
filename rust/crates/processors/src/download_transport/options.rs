//! Decides whether a generated curl invocation can run on the native engine.
//!
//! The inspector reads the exact command the downloader built, argument by
//! argument. It admits only the options listed in [`inspect`] with values it
//! has verified, and returns a reason for everything else so the original
//! command runs unchanged. It never edits or rebuilds that command.
use reqwest::header::{HeaderName, HeaderValue};
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// The Python downloader's write-out format, which reports the final URL.
pub(crate) const EFFECTIVE_URL_FORMAT: &str = "\nAUTOPKG_EFFECTIVE_URL:%{url_effective}";

/// Why a request stays on curl. Reasons name options, never their values.
pub(crate) type Reason = String;

/// The user agent a request sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserAgent {
    /// The user agent of the curl executable the fallback would run.
    Curl,
    /// A recipe-supplied user agent.
    Custom(HeaderValue),
    /// No user agent, as `--user-agent ""` requests.
    Omitted,
}

/// Where a request's trusted roots come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Trust {
    /// PEM files and hashed certificate directories.
    Files {
        files: Vec<PathBuf>,
        directories: Vec<PathBuf>,
    },
    /// The operating system's verifier.
    Platform,
}

/// A request the native engine has verified it can perform as curl would.
#[derive(Debug, Clone)]
pub(crate) struct NativeRequest {
    pub(crate) url: url::Url,
    pub(crate) head: bool,
    pub(crate) fail: bool,
    pub(crate) follow: bool,
    pub(crate) low_speed_time: Option<Duration>,
    pub(crate) output: Option<PathBuf>,
    pub(crate) user_agent: UserAgent,
    /// Headers in the order curl sends them, after recipe overrides.
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) write_out: bool,
    pub(crate) trust: Trust,
    /// Whether range requests may split the transfer.
    pub(crate) chunkable: bool,
}

/// Headers whose semantics the engine does not reproduce. Requests that set
/// or remove them stay on curl.
const UNSUPPORTED_HEADERS: [&str; 14] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "te",
    "trailer",
    "upgrade",
    "expect",
    "range",
    "if-range",
    "accept-encoding",
    "proxy-authorization",
    "proxy-connection",
];

/// Conditional headers that pin a transfer to one representation. Only the
/// downloader's own cache validators are compatible with range requests.
const CONDITIONAL_HEADERS: [&str; 2] = ["if-match", "if-unmodified-since"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Option_ {
    Flag(Flag),
    Value(ValueOption),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flag {
    Ignored,
    Location,
    Fail,
    Head,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueOption {
    DumpHeader,
    SpeedTime,
    Url,
    Output,
    Header,
    UserAgent,
    Referer,
    CaCert,
    CaPath,
    WriteOut,
}

fn long(name: &str, python: bool) -> Option<Option_> {
    use Flag::*;
    use ValueOption::*;
    Some(match name {
        "--silent" | "--show-error" | "--no-buffer" => Option_::Flag(Ignored),
        "--location" => Option_::Flag(Location),
        "--fail" => Option_::Flag(Fail),
        "--head" => Option_::Flag(Head),
        "--dump-header" => Option_::Value(DumpHeader),
        "--speed-time" => Option_::Value(SpeedTime),
        "--url" => Option_::Value(Url),
        "--output" => Option_::Value(Output),
        "--header" => Option_::Value(Header),
        "--user-agent" => Option_::Value(UserAgent),
        "--referer" => Option_::Value(Referer),
        // Only the Python downloader generates these; recipe curl_opts that
        // name them stay on curl.
        "--cacert" if python => Option_::Value(CaCert),
        "--capath" if python => Option_::Value(CaPath),
        "--write-out" if python => Option_::Value(WriteOut),
        _ => return None,
    })
}

fn short(letter: char) -> Option<Option_> {
    use Flag::*;
    use ValueOption::*;
    Some(match letter {
        's' | 'S' | 'N' => Option_::Flag(Ignored),
        'L' => Option_::Flag(Location),
        'f' => Option_::Flag(Fail),
        'I' => Option_::Flag(Head),
        'D' => Option_::Value(DumpHeader),
        'y' => Option_::Value(SpeedTime),
        'o' => Option_::Value(Output),
        'H' => Option_::Value(Header),
        'A' => Option_::Value(UserAgent),
        'e' => Option_::Value(Referer),
        _ => return None,
    })
}

/// Splits the arguments into options and values the way curl does: long
/// options take the next argument, and a short-option group ends at the
/// first letter that takes a value, which uses the rest of the group or the
/// next argument.
fn tokens(arguments: &[String], python: bool) -> Result<Vec<(Option_, Option<String>)>, Reason> {
    let mut parsed = Vec::new();
    let mut iter = arguments.iter();
    while let Some(argument) = iter.next() {
        if let Some(rest) = argument.strip_prefix("--") {
            if rest.is_empty() {
                return Err("the -- separator".into());
            }
            let option = long(argument, python)
                .ok_or_else(|| format!("unverified option {}", redacted_name(argument)))?;
            let value = match option {
                Option_::Flag(_) => None,
                Option_::Value(_) => Some(
                    iter.next()
                        .ok_or_else(|| format!("{argument} without a value"))?
                        .clone(),
                ),
            };
            parsed.push((option, value));
        } else if let Some(group) = argument.strip_prefix('-').filter(|g| !g.is_empty()) {
            let letters = group.char_indices();
            for (index, letter) in letters {
                let option = short(letter)
                    .ok_or_else(|| format!("unverified option -{}", redacted_letter(letter)))?;
                match option {
                    Option_::Flag(_) => parsed.push((option, None)),
                    Option_::Value(_) => {
                        let attached = &group[index + letter.len_utf8()..];
                        let value = if attached.is_empty() {
                            iter.next()
                                .ok_or_else(|| format!("-{letter} without a value"))?
                                .clone()
                        } else {
                            attached.to_string()
                        };
                        parsed.push((option, Some(value)));
                        break;
                    }
                }
            }
        } else {
            return Err("a positional argument".into());
        }
    }
    Ok(parsed)
}

/// Option names are safe to log; anything else might be a value.
fn redacted_name(argument: &str) -> String {
    if argument.len() <= 40
        && argument[2..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        argument.to_string()
    } else {
        "(unprintable)".into()
    }
}
fn redacted_letter(letter: char) -> String {
    if letter.is_ascii_alphanumeric() {
        letter.to_string()
    } else {
        "?".into()
    }
}

/// A recipe header after curl's rules: `Name: value` sends a header, and
/// `Name:` with nothing after the colon removes curl's own header.
fn header(line: &str) -> Result<(HeaderName, Option<HeaderValue>), Reason> {
    if line.starts_with('@') {
        return Err("a header read from a file".into());
    }
    if line.contains(['\r', '\n']) {
        return Err("a header with a line break".into());
    }
    let Some((name, value)) = line.split_once(':') else {
        return Err("a header without a colon".into());
    };
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| "a header with an invalid name".to_string())?;
    if UNSUPPORTED_HEADERS.contains(&name.as_str()) {
        return Err(format!("the {} header", name.as_str()));
    }
    let value = value.trim_matches([' ', '\t']);
    if value.is_empty() {
        if name == reqwest::header::ACCEPT {
            return Err("removing the accept header".into());
        }
        return Ok((name, None));
    }
    let value = HeaderValue::from_str(value)
        .map_err(|_| format!("a {} header with an invalid value", name.as_str()))?;
    Ok((name, Some(value)))
}

fn url(text: &str) -> Result<url::Url, Reason> {
    // curl expands [] and {} ranges, and handles spaces and non-ASCII
    // characters itself. Leave those spellings to curl.
    if !text.is_ascii()
        || text
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b"[]{}".contains(&b))
    {
        return Err("a URL that curl would rewrite or expand".into());
    }
    let parsed = url::Url::parse(text).map_err(|_| "a URL curl must interpret".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "the {} scheme",
            redacted_letterless(parsed.scheme())
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("credentials in the URL".into());
    }
    if parsed.host_str().is_none() {
        return Err("a URL without a host".into());
    }
    Ok(parsed)
}
fn redacted_letterless(scheme: &str) -> &str {
    if scheme.len() <= 16 {
        scheme
    } else {
        "unrecognized"
    }
}

/// Inspects the downloader's curl command. Returns the native request, or
/// the reason the original command must run.
pub(crate) fn inspect(command: &Command, python: bool) -> Result<NativeRequest, Reason> {
    let mut arguments = Vec::new();
    for argument in command.get_args() {
        arguments.push(
            argument
                .to_str()
                .ok_or("an argument that is not UTF-8")?
                .to_string(),
        );
    }
    let mut urls = Vec::new();
    let mut head = false;
    let mut fail = false;
    let mut follow = false;
    let mut low_speed_time = None;
    let mut output = None;
    let mut custom = Vec::new();
    let mut user_agent = UserAgent::Curl;
    let mut referer = None;
    let mut files = Vec::new();
    let mut directories = Vec::new();
    let mut write_out = false;
    for (option, value) in tokens(&arguments, python)? {
        let value = value.unwrap_or_default();
        match option {
            Option_::Flag(Flag::Ignored) => {}
            Option_::Flag(Flag::Location) => follow = true,
            Option_::Flag(Flag::Fail) => fail = true,
            Option_::Flag(Flag::Head) => head = true,
            Option_::Value(ValueOption::DumpHeader) if value == "-" => {}
            Option_::Value(ValueOption::DumpHeader) => return Err("--dump-header to a file".into()),
            Option_::Value(ValueOption::SpeedTime) => {
                let seconds = value
                    .parse::<u64>()
                    .ok()
                    .filter(|s| (1..=86_400).contains(s))
                    .ok_or("an unverified --speed-time value")?;
                low_speed_time = Some(Duration::from_secs(seconds));
            }
            Option_::Value(ValueOption::Url) => urls.push(value),
            Option_::Value(ValueOption::Output) => {
                if output.is_some() || value == "-" || value.is_empty() {
                    return Err("an unverified --output".into());
                }
                output = Some(PathBuf::from(value));
            }
            Option_::Value(ValueOption::Header) => custom.push(header(&value)?),
            Option_::Value(ValueOption::UserAgent) => {
                user_agent = if value.is_empty() {
                    UserAgent::Omitted
                } else {
                    UserAgent::Custom(
                        HeaderValue::from_str(&value)
                            .map_err(|_| "an invalid --user-agent value".to_string())?,
                    )
                };
            }
            Option_::Value(ValueOption::Referer) => {
                if value.ends_with(";auto") {
                    return Err("an automatic --referer".into());
                }
                referer = if value.is_empty() {
                    None
                } else {
                    Some(
                        HeaderValue::from_str(&value)
                            .map_err(|_| "an invalid --referer value".to_string())?,
                    )
                };
            }
            // curl uses the last --cacert, and every --capath directory.
            Option_::Value(ValueOption::CaCert) => files = vec![PathBuf::from(value)],
            Option_::Value(ValueOption::CaPath) => directories = vec![PathBuf::from(value)],
            Option_::Value(ValueOption::WriteOut) if value == EFFECTIVE_URL_FORMAT => {
                write_out = true
            }
            Option_::Value(ValueOption::WriteOut) => return Err("a custom --write-out".into()),
        }
    }
    let [url_text] = urls.as_slice() else {
        return Err("more or fewer than one URL".into());
    };
    let url = url(url_text)?;
    if !follow {
        return Err("a request without --location".into());
    }
    if !head && output.is_none() {
        return Err("a body written to standard output".into());
    }
    if head && (output.is_some() || fail) {
        return Err("an unverified --head combination".into());
    }
    let trust = trust(command, python, files, directories)?;
    // Recipe headers replace curl's own headers of the same name, in any
    // order. Every recipe header with a value is sent, including repeats.
    let overridden = |name: &HeaderName| custom.iter().any(|(n, _)| n == name);
    if overridden(&reqwest::header::USER_AGENT) {
        user_agent = UserAgent::Omitted;
    }
    let mut headers = Vec::new();
    if let Some(referer) = referer.filter(|_| !overridden(&reqwest::header::REFERER)) {
        headers.push((reqwest::header::REFERER, referer));
    }
    let chunkable = !head
        && !custom
            .iter()
            .any(|(name, _)| CONDITIONAL_HEADERS.contains(&name.as_str()));
    headers.extend(
        custom
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?))),
    );
    Ok(NativeRequest {
        url,
        head,
        fail,
        follow,
        low_speed_time,
        output,
        user_agent,
        headers,
        write_out,
        trust,
        chunkable,
    })
}

fn variable(command: &Command, name: &str) -> Option<std::ffi::OsString> {
    for (key, value) in command.get_envs() {
        if key == OsStr::new(name) {
            return value.map(OsStr::to_owned);
        }
    }
    std::env::var_os(name)
}

fn trust(
    command: &Command,
    python: bool,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
) -> Result<Trust, Reason> {
    for (key, _) in command.get_envs() {
        if key != OsStr::new("SSL_CERT_FILE") {
            return Err("a changed curl environment".into());
        }
    }
    if python {
        if files.is_empty() {
            return Err("a Python request without a CA bundle".into());
        }
        return Ok(Trust::Files { files, directories });
    }
    // The curl tool reads these before its compiled-in defaults.
    for name in ["CURL_CA_BUNDLE", "SSL_CERT_DIR"] {
        if variable(command, name).is_some_and(|v| !v.is_empty()) {
            return Err(format!("the {name} environment variable"));
        }
    }
    match variable(command, "SSL_CERT_FILE").filter(|v| !v.is_empty()) {
        Some(file) => Ok(Trust::Files {
            files: vec![PathBuf::from(file)],
            directories: Vec::new(),
        }),
        // macOS always sets a bundle for curl; see download_trust.rs.
        None if cfg!(target_os = "macos") => Err("macOS curl without a CA bundle".into()),
        None => Ok(Trust::Platform),
    }
}

/// Configuration curl reads that the engine does not interpret. Any of it
/// keeps every request on curl.
pub(crate) fn environment(program: &OsStr) -> Result<(), Reason> {
    for name in [
        "http_proxy",
        "HTTP_PROXY",
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "CURL_SSL_BACKEND",
    ] {
        if std::env::var_os(name).is_some_and(|v| !v.is_empty()) {
            return Err(format!("the {name} environment variable"));
        }
    }
    if curl_config(program).is_some() {
        return Err("a curl configuration file".into());
    }
    if cfg!(unix) && std::env::var_os("HOME").is_none() {
        return Err("an unknown home directory".into());
    }
    Ok(())
}

/// The default configuration files curl looks for, on any platform. curl
/// reads the first one it finds; any of them disqualifies native transfer.
fn curl_config(program: &OsStr) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    for (variable, suffix) in [
        ("CURL_HOME", ""),
        ("XDG_CONFIG_HOME", ""),
        ("HOME", ""),
        ("USERPROFILE", ""),
        ("APPDATA", ""),
        ("USERPROFILE", "Application Data"),
    ] {
        if let Some(base) = std::env::var_os(variable).filter(|v| !v.is_empty()) {
            let directory = PathBuf::from(base).join(suffix);
            for name in [".curlrc", "_curlrc", "curlrc"] {
                candidates.push(directory.join(name));
            }
        }
    }
    if cfg!(windows) {
        if let Some(directory) = std::path::Path::new(program).parent() {
            candidates.push(directory.join(".curlrc"));
            candidates.push(directory.join("_curlrc"));
        }
    }
    candidates.into_iter().find(|path| path.exists())
}
