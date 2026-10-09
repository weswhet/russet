//! `AdobeAcrobatProUpdateInfoProvider`: find the latest Acrobat Pro update.
//! A native port of the autopkg/recipes processor (Apache-2.0).
use crate::community_legacy::{fetch, get, output, Result, TypedResult};
use plist::{Dictionary, Value};

pub(crate) fn acrobat<F>(env: &mut Dictionary, mut fetcher: F) -> TypedResult<()>
where
    F: FnMut(&Dictionary, &str) -> Result<String>,
{
    let major = crate::string(env, "major_version")?.to_owned();
    if !["9", "10", "11"].contains(&major.as_str()) {
        return Err(format!("major_version {major} not one of those supported: 9, 10, 11").into());
    }
    let os = get(env, "target_os", "10.9").to_owned();
    let parts: Vec<_> = os.split('.').collect();
    if parts.len() < 2 {
        return Err(format!("OS X Version {os} not recognised").into());
    }
    if parts[0] != "10" {
        return Err(format!("Major OS Version {} is not supported", parts[0]).into());
    }
    let minor: i32 = parts[1]
        .parse()
        .map_err(|_| format!("OS X Version {os} not recognised"))?;
    if minor < 6 {
        return Err(format!("Minor OS Version {} is not supported", parts[1]).into());
    }
    let substitute = |s: &str| {
        s.replace("{PROD}", "com_adobe_Acrobat_Pro")
            .replace("{PROD_ARCH}", "univ")
            .replace("{MAJREV}", &major)
            .replace("{OS_VER_MAJ}", parts[0])
            .replace("{OS_VER_MIN}", parts[1])
    };
    let base = "https://armmf.adobe.com/arm-manifests/mac";
    let mut template = fetcher(env, &format!("{base}/{major}/manifest_url_template.txt"))?;
    let version = get(env, "version", "latest");
    if version != "latest" {
        template = regex::Regex::new(r"\d+\.\d+\.\d+")
            .unwrap()
            .replace_all(&template, regex::NoExpand(version))
            .into_owned();
    }
    let mut manifest = |url: &str| -> Result<Dictionary> {
        let text = fetcher(env, url)?;
        let value = Value::from_reader(std::io::Cursor::new(text.as_bytes()))
            .map_err(|e| format!("Can't parse manifest plist at {url}: {e}"))?;
        let d = value
            .into_dictionary()
            .ok_or_else(|| format!("Can't parse manifest plist at {url}: expected dictionary"))?;
        if !d.contains_key("PatchURL") {
            return Err(format!("Manifest plist key 'PatchURL' not found at {url}"));
        }
        Ok(d)
    };
    let data = manifest(&substitute(&format!("{base}{template}")))?;
    let previous = crate::string(&data, "PreviousURLTemplate")?;
    if previous == "noTemplate" {
        return Err(crate::ExecutionFailure::unexpected(
            "local variable 'prev_version' referenced before assignment",
        ));
    }
    let previous = manifest(&substitute(&format!("{base}{previous}")))?;
    let previous = crate::string(&previous, "BuildNumber")?;
    let version = crate::string(&data, "BuildNumber")?;
    let mut info = Dictionary::new();
    if !regex::Regex::new(r"\.[0]+\.[0]+")
        .unwrap()
        .is_match(previous)
    {
        let name = get(env, "munki_update_name", "");
        let name = if name.is_empty() {
            format!("AdobeAcrobatPro{major}_Update")
        } else {
            name.into()
        };
        output(1, format!("Update requires previous version: {previous}"));
        info.insert(
            "requires".into(),
            Value::Array(vec![format!("{name}-{previous}").into()]),
        );
    }
    info.insert("minimum_os_version".into(), format!("{os}.0").into());
    info.insert("version".into(), version.into());
    env.insert("additional_pkginfo".into(), info.into());
    env.insert("version".into(), version.into());
    env.insert(
        "url".into(),
        format!(
            "http://armdl.adobe.com{}",
            crate::string(&data, "PatchURL")?
        )
        .into(),
    );
    output(1, format!("Found URL {}", crate::string(env, "url")?));
    Ok(())
}

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    acrobat(env, fetch).map_err(|e| e.message)
}

/// The standalone form, which keeps Python's runtime-exception boundary.
pub(crate) fn execute_typed(env: &mut Dictionary) -> TypedResult<()> {
    acrobat(env, fetch)
}
