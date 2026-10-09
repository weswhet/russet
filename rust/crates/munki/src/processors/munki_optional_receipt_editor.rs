//! `MunkiOptionalReceiptEditor`: mark package receipts as optional in a
//! pkginfo in the Munki repo.
//!
//! Inputs and outputs: run `russet processor-info MunkiOptionalReceiptEditor`, or see
//! `MunkiOptionalReceiptEditor` in `compatibility/reference.json`.
use crate::{string, FileRepo};
use plist::{Dictionary, Value};
use std::path::Path;

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    let path = string(env, "pkginfo_repo_path")?.to_owned();
    if path.is_empty() {
        autopkg_platform::processor_output(1, "No pkginfo_repo_path specified, skipping");
        if !env.contains_key("munki_info") {
            env.insert("munki_info".into(), Value::Dictionary(Dictionary::new()));
        }
        return Ok(());
    }
    let ids = env
        .get("pkg_ids_set_optional_true")
        .and_then(Value::as_array)
        .ok_or("pkg_ids_set_optional_true must be an array")?;
    let mut info = match env
        .get("munki_info")
        .and_then(Value::as_dictionary)
        .filter(|d| !d.is_empty())
    {
        Some(info) => info.clone(),
        None => Value::from_file(&path)
            .map_err(|e| e.to_string())?
            .into_dictionary()
            .ok_or("pkginfo must be a dictionary")?,
    };
    let receipts = info
        .get_mut("receipts")
        .and_then(Value::as_array_mut)
        .ok_or("pkginfo does not contain any receipts")?;
    let mut changed = false;
    for receipt in receipts {
        let receipt = receipt
            .as_dictionary_mut()
            .ok_or("Invalid receipt dictionary")?;
        let id = receipt
            .get("packageid")
            .ok_or("Receipt does not contain packageid")?;
        if ids.contains(id) {
            autopkg_platform::processor_output(
                1,
                format!("Setting package ID {} as optional", plist::python_str(id)),
            );
            receipt.insert("optional".into(), Value::Boolean(true));
            changed = true;
        }
    }
    if changed {
        autopkg_platform::processor_output(1, format!("Writing pkginfo to {path}"));
        FileRepo::new(string(env, "MUNKI_REPO")?).put_pkginfo(&info, Path::new(&path))?;
    } else {
        autopkg_platform::processor_output(1, "No receipts modified, nothing to do");
    }
    env.insert("munki_info".into(), Value::Dictionary(info));
    Ok(())
}
