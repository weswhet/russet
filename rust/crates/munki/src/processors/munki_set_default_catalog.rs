//! `MunkiSetDefaultCatalog`: set the pkginfo's catalogs to munkiimport's
//! default catalog, when one is set.
use crate::truthy;
use plist::{Dictionary, Value};

pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    if !env.contains_key("pkginfo") {
        env.insert("pkginfo".into(), Value::Dictionary(Dictionary::new()));
    }
    let mut changed = false;
    if let Some(catalog) =
        autopkg_platform::preference("com.googlecode.munki.munkiimport", "default_catalog")?
    {
        let nonempty = truthy(&catalog);
        if nonempty {
            let message = format!(
                "Updated target catalogs into pkginfo with {}",
                plist::python_str(&catalog)
            );
            env.get_mut("pkginfo")
                .and_then(Value::as_dictionary_mut)
                .ok_or("pkginfo must be a dictionary")?
                .insert("catalogs".into(), Value::Array(vec![catalog]));
            changed = true;
            autopkg_platform::processor_output(1, message);
        }
    }
    if !changed {
        autopkg_platform::processor_output(1, "No default catalogs found, nothing changed");
    }
    Ok(())
}
