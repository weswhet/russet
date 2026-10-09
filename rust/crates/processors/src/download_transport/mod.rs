//! The transport for URL download processors. Recipe policy stays in
//! `downloader.rs`, which builds the curl command; this module runs it.
//!
//! Two backends perform a transfer. Russet's native engine runs a request
//! when every argument and the environment have verified equivalent
//! behavior. Everything else, and every request when the
//! `UseRussetDownloader` preference is false, runs the original curl
//! command unchanged. The choice is made before any network activity, and a
//! native request is never replayed through curl.
mod chunks;
mod curl;
mod native;
mod options;
#[cfg(test)]
mod tests;

use plist::{Dictionary, Value};
use std::process::Command;

#[cfg(test)]
pub(super) use curl::curl_stderr;
pub(super) use curl::{Headers, TransportFailure};

/// The preference that selects the native engine. It defaults to true.
pub(crate) const PREFERENCE: &str = "UseRussetDownloader";

/// The backend a recipe's preferences allow.
#[derive(Clone, Debug)]
pub(super) struct Policy {
    /// `Err` holds the reason every request uses curl.
    native: Result<(), String>,
}

fn preference(env: &Dictionary, key: &str) -> Result<Option<Value>, String> {
    match env.get(key) {
        Some(value) => Ok(Some(value.clone())),
        None => autopkg_platform::preference("com.github.autopkg", key),
    }
}

fn enabled(value: &Value) -> Result<bool, String> {
    let invalid = || format!("{PREFERENCE} must be a boolean or boolean-like string");
    match value {
        Value::Boolean(b) => Ok(*b),
        Value::Integer(n) => match n.as_signed() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(invalid()),
        },
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" => Ok(false),
            _ => Err(invalid()),
        },
        _ => Err(invalid()),
    }
}

/// Reads the backend preferences the same way `CURL_PATH` is read: the
/// recipe environment first, which holds preferences and `AUTOPKG_`
/// variables, then the macOS preference domain.
pub(super) fn policy(env: &Dictionary) -> Result<Policy, crate::ExecutionFailure> {
    let native = match preference(env, PREFERENCE)? {
        Some(value) if !enabled(&value)? => Err(format!("{PREFERENCE} is false")),
        _ if preference(env, "CURL_PATH")?.is_some() => Err("CURL_PATH is set".into()),
        _ => Ok(()),
    };
    Ok(Policy { native })
}

fn debug(message: impl std::fmt::Display) {
    if std::env::var_os("AUTOPKG_RS_DEBUG").is_some() {
        autopkg_platform::text_eprintln!("Download backend: {message}");
    }
}

/// Runs a download's command with the selected backend, returning the
/// response headers and, for the Python downloader, the effective URL.
pub(super) fn run(
    command: Command,
    python: bool,
    policy: &Policy,
) -> Result<(Headers, String), TransportFailure> {
    run_with(command, python, policy, chunks::ChunkPolicy::default())
}

fn run_with(
    command: Command,
    python: bool,
    policy: &Policy,
    chunking: chunks::ChunkPolicy,
) -> Result<(Headers, String), TransportFailure> {
    let prepared = policy
        .native
        .clone()
        .and_then(|()| options::environment(command.get_program()))
        .and_then(|()| options::inspect(&command, python))
        .and_then(|request| native::prepare(request, command.get_program()));
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(reason) => {
            debug(format_args!("curl, because of {reason}"));
            return curl::execute(command, python);
        }
    };
    let started = std::time::Instant::now();
    match native::execute(&prepared, chunking) {
        Ok(transfer) => {
            debug(format_args!(
                "Russet downloader, {} in {:.2?}",
                transfer.detail,
                started.elapsed()
            ));
            if !python {
                autopkg_platform::processor_output(
                    4,
                    format!(
                        "Russet downloader request for curl command: {}",
                        curl::arguments(&command)
                    ),
                );
            }
            Ok((transfer.headers, transfer.effective))
        }
        Err(failure) => {
            debug(format_args!(
                "Russet downloader, failed with curl code {} in {:.2?}",
                failure.code,
                started.elapsed()
            ));
            let message = failure.curl_message();
            curl::check_exit(python, Some(failure.code), failure.headers.clone(), || {
                message
            })?;
            Ok((failure.headers, failure.effective))
        }
    }
}
