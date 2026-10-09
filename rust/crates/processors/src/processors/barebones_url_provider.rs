//! `BarebonesURLProvider`: find the download URL and version for BBEdit or
//! Yojimbo. A native port of the autopkg/recipes processor (Apache-2.0).
use crate::community_modern::{fetch, get, output};
use crate::processors::sparkle_update_info_provider::version_cmp;
use crate::{string, Result};
use plist::{Dictionary, Value};
use std::cmp::Ordering;

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let product = string(env, "product_name")?;
    let url = match product {
        "bbedit" => "https://versioncheck.barebones.com/BBEdit.xml",
        "yojimbo" => "https://versioncheck.barebones.com/Yojimbo.xml",
        _ => {
            return Err(format!(
                "product_name {product} is invalid; it must be one of: bbedit, yojimbo"
            ))
        }
    };
    let data = fetch(env, url, None)?;
    barebones_metadata(env, &data)
}
pub(crate) fn barebones_metadata(env: &mut Dictionary, data: &str) -> Result<()> {
    let manifest = Value::from_reader(std::io::Cursor::new(data.as_bytes()))
        .map_err(|e| format!("Unexpected error parsing manifest as a plist: '{e}'"))?;
    let entries = manifest
        .as_dictionary()
        .and_then(|d| d.get("SUFeedEntries"))
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or("Expected 'SUFeedEntries' manifest key wasn't found.")?;
    let mut latest: Option<&Dictionary> = None;
    for entry in entries {
        let entry = entry
            .as_dictionary()
            .ok_or("Feed entry must be a dictionary")?;
        let version = string(entry, "SUFeedEntryShortVersionString")?;
        // Python stable sort followed by [-1] selects the last equal version.
        if latest.is_none_or(|last| {
            version_cmp(version, get(last, "SUFeedEntryShortVersionString", "")) != Ordering::Less
        }) {
            latest = Some(entry);
        }
    }
    let latest = latest.unwrap();
    for (from, to) in [
        ("SUFeedEntryShortVersionString", "version"),
        ("SUFeedEntryDownloadURL", "url"),
        ("SUFeedEntryMinimumSystemVersion", "minimum_os_version"),
    ] {
        env.insert(
            to.into(),
            latest
                .get(from)
                .ok_or_else(|| format!("Missing {from}"))?
                .clone(),
        );
    }
    output(format!("Found URL {}", string(env, "url")?));
    Ok(())
}
