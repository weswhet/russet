//! `URLDownloaderPython`: `URLDownloader` with Python's request behavior. The
//! two share one implementation, which takes the processor's name to choose.
//!
//! Inputs and outputs: run `russet processor-info URLDownloaderPython`, or see
//! `URLDownloaderPython` in `compatibility/reference.json`.
use super::url_downloader;
use crate::{ExecutionFailure, Result};
use plist::Dictionary;

const NAME: &str = "URLDownloaderPython";

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    url_downloader::execute(NAME, env)
}

pub(crate) fn execute_typed(env: &mut Dictionary) -> std::result::Result<(), ExecutionFailure> {
    url_downloader::execute_typed(NAME, env)
}
