//! The transport for URL download processors. Recipe policy stays in
//! `downloader.rs`, which builds the command; this module runs it.
//!
//! Curl is the only backend. Requests reach it through [`run`], so a second
//! backend can be selected here without changing the processors.
mod curl;

use std::process::Command;

#[cfg(test)]
pub(super) use curl::curl_stderr;
pub(super) use curl::{Headers, TransportFailure};

/// Runs a download's command with the backend in use, returning the response
/// headers and effective URL.
pub(super) fn run(command: Command, python: bool) -> Result<(Headers, String), TransportFailure> {
    curl::execute(command, python)
}
