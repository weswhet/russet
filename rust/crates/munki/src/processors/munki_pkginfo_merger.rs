//! `MunkiPkginfoMerger`: merge additional keys into the pkginfo.
use plist::{Dictionary, Value};

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    let additional = env
        .get("additional_pkginfo")
        .and_then(Value::as_dictionary)
        .ok_or("additional_pkginfo must be a dictionary")?
        .clone();
    if !env.contains_key("pkginfo") {
        env.insert("pkginfo".into(), Value::Dictionary(Dictionary::new()));
    }
    let info = env
        .get_mut("pkginfo")
        .and_then(Value::as_dictionary_mut)
        .ok_or("pkginfo must be a dictionary")?;
    let message = format!(
        "Merged {} into pkginfo",
        plist::python_repr(&Value::Dictionary(additional.clone()))
    );
    info.extend(additional);
    autopkg_platform::processor_output(1, message);
    Ok(())
}
