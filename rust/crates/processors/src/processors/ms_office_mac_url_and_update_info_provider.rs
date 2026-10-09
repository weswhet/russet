//! `MSOfficeMacURLandUpdateInfoProvider`: find Microsoft Office for Mac and
//! Edge downloads through Microsoft AutoUpdate's feed. A native port of the
//! autopkg/recipes processor (Apache-2.0).
use crate::community_modern::{fetch, get, output};
use crate::processors::sparkle_update_info_provider::version_cmp;
use crate::{string, truth, Result};
use plist::{Dictionary, Value};
use std::cmp::Ordering;

pub(crate) const PRODUCTS: &str = r#"{"Excel2016":{"id":"XCEL15","path":"/Applications/Microsoft Excel.app"},"Excel2019":{"bundle_id":"com.microsoft.Excel","id":"XCEL2019","path":"/Applications/Microsoft Excel.app","minimum_os":"10.12","minimum_update_version":"16.17"},"OneNote2016":{"id":"ONMC15","path":"/Applications/Microsoft OneNote.app"},"OneNote2019":{"bundle_id":"com.microsoft.onenote.mac","id":"ONMC2019","path":"/Applications/Microsoft OneNote.app","minimum_os":"10.12","minimum_update_version":"16.17"},"Outlook2016":{"id":"OPIM15","path":"/Applications/Microsoft Outlook.app"},"Outlook2019":{"bundle_id":"com.microsoft.Outlook","id":"OPIM2019","path":"/Applications/Microsoft Outlook.app","minimum_os":"10.12","minimum_update_version":"16.17"},"PowerPoint2016":{"id":"PPT315","path":"/Applications/Microsoft PowerPoint.app"},"PowerPoint2019":{"bundle_id":"com.microsoft.PowerPoint","id":"PPT32019","path":"/Applications/Microsoft PowerPoint.app","minimum_os":"10.12","minimum_update_version":"16.17"},"Word2016":{"id":"MSWD15","path":"/Applications/Microsoft Word.app"},"Word2019":{"bundle_id":"com.microsoft.Word","id":"MSWD2019","path":"/Applications/Microsoft Word.app","minimum_os":"10.12","minimum_update_version":"16.17"},"SkypeForBusiness":{"id":"MSFB16","path":"/Applications/Skype for Business.app"},"AutoUpdate03":{"id":"MSau03","path":"/Library/Application Support/Microsoft/MAU2.0/Microsoft AutoUpdate.app"},"AutoUpdate04":{"id":"MSau04","path":"/Library/Application Support/Microsoft/MAU2.0/Microsoft AutoUpdate.app"},"DefenderATP":{"id":"WDAV00","path":"/Applications/Microsoft Defender ATP.app","minimum_os":"10.12"},"Edge":{"id":"EDGE01","path":"/Applications/Microsoft Edge.app","minimum_os":"10.11"},"Teams":{"id":"TEAMS10","path":"/Applications/Microsoft Teams classic.app","minimum_os":"10.11"},"Teams2":{"id":"TEAMS21","path":"/Applications/Microsoft Teams.app","minimum_os":"12.0"},"CompanyPortal":{"id":"IMCP01","path":"/Applications/Company Portal.app","minimum_os":"10.15"},"OneDrive":{"id":"ONDR18","path":"/Applications/OneDrive.app","minimum_os":"10.15"},"RemoteDesktop":{"id":"MSRD10","path":"/Applications/Windows App.app","minimum_os":"12.0"}}"#;
const EDGE_URL: &str = "https://edgeupdates.microsoft.com/api/products?view=enterprise";
pub(crate) fn edge_metadata(
    env: &mut Dictionary,
    data: &str,
    channel: &str,
    bundle: &str,
    path: &str,
) -> Result<()> {
    let products: serde_json::Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    let text = |v: &serde_json::Value, k: &str| {
        v.get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let product = products
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|p| text(p, "Product").eq_ignore_ascii_case(channel))
        })
        .ok_or_else(|| {
            format!("Could not find Edge channel '{channel}' in Enterprise API response.")
        })?;
    let mut selected: Option<(&serde_json::Value, &serde_json::Value)> = None;
    for release in product
        .get("Releases")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        if !text(release, "Platform").eq_ignore_ascii_case("macos")
            || !text(release, "Architecture").eq_ignore_ascii_case("universal")
        {
            continue;
        }
        if let Some(artifact) = release
            .get("Artifacts")
            .and_then(serde_json::Value::as_array)
            .and_then(|a| {
                a.iter().find(|a| {
                    text(a, "ArtifactName").eq_ignore_ascii_case("pkg")
                        || text(a, "Location").to_lowercase().ends_with(".pkg")
                })
            })
        {
            let version = text(release, "ProductVersion");
            if !version.is_empty()
                && !text(artifact, "Location").is_empty()
                && selected.is_none_or(|(r, _)| {
                    version_cmp(&version, &text(r, "ProductVersion")) == Ordering::Greater
                })
            {
                selected = Some((release, artifact));
            }
        }
    }
    let (release, artifact) = selected.ok_or_else(|| {
        format!("Could not find a macOS universal pkg release for Edge channel '{channel}'.")
    })?;
    let version = text(release, "ProductVersion");
    let url = text(artifact, "Location").trim().to_owned();
    let install = Dictionary::from_iter([
        ("CFBundleIdentifier", bundle),
        ("CFBundleShortVersionString", version.as_str()),
        ("path", path),
        ("type", "application"),
    ]);
    let mut pkginfo = Dictionary::new();
    pkginfo.insert("installs".into(), Value::Array(vec![install.into()]));
    env.insert("version".into(), version.clone().into());
    env.insert("minimum_version_for_delta".into(), "".into());
    env.insert("url".into(), url.clone().into());
    env.insert("additional_pkginfo".into(), pkginfo.into());
    output(format!("Found Edge {channel} version {version}"));
    output(format!("Found URL {url}"));
    output(format!(
        "Additional pkginfo: {}",
        plist::python_repr(&env["additional_pkginfo"])
    ));
    Ok(())
}
pub(crate) fn office_metadata(
    env: &mut Dictionary,
    data: &str,
    product: &str,
    spec: &serde_json::Value,
    endpoint: &str,
) -> Result<()> {
    let metadata =
        Value::from_reader(std::io::Cursor::new(data.as_bytes())).map_err(|e| e.to_string())?;
    let entries = metadata.as_array().ok_or_else(||format!("No update metadata returned for product '{product}' from {endpoint}. This product may no longer be published to the update feed."))?;
    let mode = get(env, "version", "latest").to_owned();
    let mut selected = None;
    for entry in entries.iter().filter_map(Value::as_dictionary) {
        let mut entry = entry.clone();
        for value in entry.values_mut() {
            if let Value::String(s) = value {
                *s = s.trim().to_owned();
            }
        }
        if truth(entry.get("FullUpdaterLocation")) == (mode == "latest-delta") {
            selected = Some(entry);
            break;
        }
    }
    let mut item = selected.ok_or("Could not find an applicable update in update metadata.")?;
    if mode == "latest-standalone" {
        let url = string(&item, "Location")?
            .strip_suffix("_Updater.pkg")
            .ok_or("Updater URL in unexpected format; cannot determine standalone URL.")?;
        item.insert("Location".into(), format!("{url}_Installer.pkg").into());
    }
    env.insert(
        "url".into(),
        item.get("Location").ok_or("Missing Location")?.clone(),
    );
    output(format!("Found URL {}", string(env, "url")?));
    output(format!("Got update: '{}'", string(&item, "Title")?));
    let baseline = spec
        .get("minimum_os")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("10.10.5");
    let mut min_os = get(&item, "Minimum OS", baseline);
    if min_os.is_empty() || version_cmp(min_os, baseline) == Ordering::Less {
        min_os = baseline;
    }
    if !["SkypeForBusiness", "Teams", "Teams2", "CompanyPortal"].contains(&product)
        && item.get("Trigger Condition")
            != Some(&Value::Array(vec!["and".into(), "Registered File".into()]))
    {
        return Err(format!(
            "Unexpected Trigger Condition in item {}: {}",
            string(&item, "Title")?,
            plist::python_repr(item.get("Trigger Condition").unwrap_or(&Value::Null))
        ));
    }
    let version = item
        .get("Update Version")
        .filter(|v| truth(Some(v)))
        .cloned()
        .unwrap_or(Value::Null);
    if !version.is_null() {
        output(format!(
            "Extracting version {} from metadata 'Update Version' key",
            string(&item, "Update Version")?
        ));
    }
    let mut install = Dictionary::new();
    install.insert("CFBundleVersion".into(), version.clone());
    install.insert("path".into(), spec["path"].as_str().unwrap().into());
    install.insert("type".into(), "application".into());
    if let Some(bundle) = spec.get("bundle_id").and_then(serde_json::Value::as_str) {
        install.insert("CFBundleIdentifier".into(), bundle.into());
    }
    let mut pkginfo = Dictionary::new();
    pkginfo.insert("minimum_os_version".into(), min_os.into());
    let mut delta = String::new();
    let mut requires = None;
    if mode == "latest-delta" {
        let expressions = item.get("Triggers").and_then(Value::as_dictionary).and_then(|d|d.get("Registered File")).and_then(Value::as_dictionary).and_then(|d|d.get("VersionsRelative")).and_then(Value::as_array).ok_or("Can't find expected VersionsRelativekeys for determining minimum update required for delta update.")?;
        for expression in expressions {
            let parts: Vec<_> = expression
                .as_string()
                .ok_or("Invalid VersionsRelative expression")?
                .split_whitespace()
                .collect();
            if parts.len() != 2 {
                return Err("Invalid VersionsRelative expression".into());
            }
            if parts[0] == ">=" {
                delta = parts[1].into();
                break;
            }
        }
        if delta.is_empty() {
            return Err("Not able to determine minimum required version for delta update.".into());
        }
        output(format!("Adding minimum required version: {delta}"));
        install.insert("minimum_update_version".into(), delta.clone().into());
        let name = string(env, "NAME")?;
        let requested = get(env, "munki_required_update_name", "");
        let name = if requested.is_empty() {
            name
        } else {
            requested
        };
        requires = Some(Value::Array(vec![format!("{name}-{delta}").into()]));
    } else if let Some(minimum) = spec
        .get("minimum_update_version")
        .and_then(serde_json::Value::as_str)
    {
        output(format!("Adding minimum required version: {minimum}"));
        install.insert("minimum_update_version".into(), minimum.into());
    }
    pkginfo.insert("installs".into(), Value::Array(vec![install.into()]));
    if let Some(requires) = requires {
        pkginfo.insert("requires".into(), requires);
    }
    if !version.is_null() {
        output(format!(
            "Extracting version {} from metadata 'Update Version' key",
            string(&item, "Update Version")?
        ));
    }
    env.insert("version".into(), version);
    env.insert("minimum_os_version".into(), min_os.into());
    env.insert("minimum_version_for_delta".into(), delta.into());
    env.insert("additional_pkginfo".into(), pkginfo.into());
    output(format!(
        "Additional pkginfo: {}",
        plist::python_repr(&env["additional_pkginfo"])
    ));
    Ok(())
}
pub(crate) fn office_is_deprecated(env: &Dictionary) -> bool {
    matches!(
        env.get("product").and_then(Value::as_string),
        Some("Excel2016" | "OneNote2016" | "Outlook2016" | "PowerPoint2016" | "Word2016")
    )
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let mode = get(env, "version", "latest");
    if !["latest", "latest-delta", "latest-standalone"].contains(&mode) {
        return Err(
            "Invalid 'version': supported values are 'latest', 'latest-delta', 'latest-standalone'"
                .into(),
        );
    }
    let product = string(env, "product")?.to_owned();
    if office_is_deprecated(env) {
        crate::processors::deprecation_warning::warn(env, format!("As of August 2026, Microsoft no longer publishes {product} updates to the Microsoft AutoUpdate feed. Office for Mac 2016 support ended in October 2020 at version 16.16.27. Please use the MS{} recipes instead.",product.replace("2016","2019")))?;
        env.insert("stop_processing_recipe".into(), true.into());
        return Ok(());
    }
    let input = get(env, "channel", "Production");
    if product == "Edge" {
        if mode != "latest" {
            return Err("Edge supports only VERSION 'latest'.".into());
        }
        let (channel,bundle,path) = match input { "Production" => ("Stable","com.microsoft.edgemac","/Applications/Microsoft Edge.app"), "InsiderSlow" => ("Beta","com.microsoft.edgemac.Beta","/Applications/Microsoft Edge Beta.app"), "InsiderFast" => ("Dev","com.microsoft.edgemac.Dev","/Applications/Microsoft Edge Dev.app"), _ => return Err("Edge CHANNEL must be one of: Production, InsiderSlow, InsiderFast. Custom UUID channels are not supported by the Edge Enterprise API.".into()) };
        output(format!("Requesting Edge Enterprise API: {EDGE_URL}"));
        let data = fetch(env, EDGE_URL, None)?;
        return edge_metadata(env, &data, channel, bundle, path);
    }
    let channel = match input { "Production" => "C1297A47-86C4-4C1F-97FA-950631F94777", "InsiderSlow" => "1ac37578-5a24-40fb-892e-b89d85b6dfaa", "InsiderFast" => "4B2D7701-0A4F-49C8-B4CB-0C2D4043F51F", value if regex::Regex::new(r"^[0-9a-fA-F]{8}-([0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$").unwrap().is_match(value) => value, _ => return Err("'channel' input variable must be one of: Production, InsiderSlow, InsiderFast or a custom uuid".into()) };
    let products: serde_json::Value = serde_json::from_str(PRODUCTS).unwrap();
    let spec = products
        .get(&product)
        .ok_or_else(|| format!("Unknown Microsoft product '{product}'"))?;
    let endpoint = format!("https://res.public.onecdn.static.microsoft/mro1cdnstorage/{channel}/MacAutoupdate/0409{}.xml",spec["id"].as_str().unwrap());
    output(format!("Requesting xml: {endpoint}"));
    let headers = Dictionary::from_iter([(
        "User-Agent",
        "Microsoft%20AutoUpdate/3.6.16080300 CFNetwork/760.6.3 Darwin/15.6.0 (x86_64)",
    )]);
    let data = fetch(env, &endpoint, Some(headers))?;
    office_metadata(env, &data, &product, spec, &endpoint)
}
