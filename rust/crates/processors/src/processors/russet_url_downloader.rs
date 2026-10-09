//! `RussetURLDownloader`: download a file into the recipe cache with Russet's
//! native HTTP engine, which splits a large download into parallel range
//! requests when the server allows it. It takes the same inputs and sets the
//! same outputs as `URLDownloader`, but never runs curl: a request the
//! engine can't reproduce exactly, such as one with an unsupported
//! `curl_opts` option or behind a proxy, fails with the reason.
//!
//! When the `UseRussetDownloader` preference is true, which is the default,
//! `URLDownloader` and `URLDownloaderPython` use this engine too, and run
//! with curl only for the requests it can't reproduce.
//!
//! Inputs and outputs: run `russet processor-info RussetURLDownloader`, or
//! see `RussetURLDownloader` in `compatibility/russet-processors.json`.
use super::url_downloader;
use crate::{ExecutionFailure, Result};
use plist::Dictionary;

pub(crate) const NAME: &str = "RussetURLDownloader";

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    url_downloader::execute(NAME, env)
}

pub(crate) fn execute_typed(env: &mut Dictionary) -> std::result::Result<(), ExecutionFailure> {
    url_downloader::execute_typed(NAME, env)
}
