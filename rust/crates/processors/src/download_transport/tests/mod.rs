//! Transport tests. Every server is local and deterministic.
mod fixture;
mod options;
mod parity;
mod ranges;

use plist::Dictionary;

/// Whether a curl executable is available for differential comparisons.
fn curl_available() -> bool {
    autopkg_platform::downloads::curl_binary(&Dictionary::new()).is_ok()
}
