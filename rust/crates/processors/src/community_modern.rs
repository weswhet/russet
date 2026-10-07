//! Native ports of the Apache-2.0 AutoPkg community providers by Allister Banks,
//! Tim Sutton, Greg Neagle, Carl Ashley, and Per Olofsson.
use super::sparkle::version_cmp;
use super::{string, truth, Result};
use plist::{Dictionary, Value};
use std::cmp::Ordering;
const PRODUCTS: &str = r#"{"Excel2016":{"id":"XCEL15","path":"/Applications/Microsoft Excel.app"},"Excel2019":{"bundle_id":"com.microsoft.Excel","id":"XCEL2019","path":"/Applications/Microsoft Excel.app","minimum_os":"10.12","minimum_update_version":"16.17"},"OneNote2016":{"id":"ONMC15","path":"/Applications/Microsoft OneNote.app"},"OneNote2019":{"bundle_id":"com.microsoft.onenote.mac","id":"ONMC2019","path":"/Applications/Microsoft OneNote.app","minimum_os":"10.12","minimum_update_version":"16.17"},"Outlook2016":{"id":"OPIM15","path":"/Applications/Microsoft Outlook.app"},"Outlook2019":{"bundle_id":"com.microsoft.Outlook","id":"OPIM2019","path":"/Applications/Microsoft Outlook.app","minimum_os":"10.12","minimum_update_version":"16.17"},"PowerPoint2016":{"id":"PPT315","path":"/Applications/Microsoft PowerPoint.app"},"PowerPoint2019":{"bundle_id":"com.microsoft.PowerPoint","id":"PPT32019","path":"/Applications/Microsoft PowerPoint.app","minimum_os":"10.12","minimum_update_version":"16.17"},"Word2016":{"id":"MSWD15","path":"/Applications/Microsoft Word.app"},"Word2019":{"bundle_id":"com.microsoft.Word","id":"MSWD2019","path":"/Applications/Microsoft Word.app","minimum_os":"10.12","minimum_update_version":"16.17"},"SkypeForBusiness":{"id":"MSFB16","path":"/Applications/Skype for Business.app"},"AutoUpdate03":{"id":"MSau03","path":"/Library/Application Support/Microsoft/MAU2.0/Microsoft AutoUpdate.app"},"AutoUpdate04":{"id":"MSau04","path":"/Library/Application Support/Microsoft/MAU2.0/Microsoft AutoUpdate.app"},"DefenderATP":{"id":"WDAV00","path":"/Applications/Microsoft Defender ATP.app","minimum_os":"10.12"},"Edge":{"id":"EDGE01","path":"/Applications/Microsoft Edge.app","minimum_os":"10.11"},"Teams":{"id":"TEAMS10","path":"/Applications/Microsoft Teams classic.app","minimum_os":"10.11"},"Teams2":{"id":"TEAMS21","path":"/Applications/Microsoft Teams.app","minimum_os":"12.0"},"CompanyPortal":{"id":"IMCP01","path":"/Applications/Company Portal.app","minimum_os":"10.15"},"OneDrive":{"id":"ONDR18","path":"/Applications/OneDrive.app","minimum_os":"10.15"},"RemoteDesktop":{"id":"MSRD10","path":"/Applications/Windows App.app","minimum_os":"12.0"}}"#;
const MOZ_URL: &str =
    "https://download.mozilla.org/?product={product_release}-ssl&os={platform}&lang={locale}";
const MOZ_VERSIONS: &str = "https://product-details.mozilla.org/1.0/{product}_versions.json";
const EDGE_URL: &str = "https://edgeupdates.microsoft.com/api/products?view=enterprise";
fn output(message: impl AsRef<str>) {
    autopkg_platform::processor_output(1, message.as_ref());
}
fn get<'a>(env: &'a Dictionary, key: &str, default: &'a str) -> &'a str {
    env.get(key).and_then(Value::as_string).unwrap_or(default)
}
fn fetch(env: &Dictionary, url: &str, headers: Option<Dictionary>) -> Result<String> {
    let mut request = env.clone();
    request.insert("url".into(), url.into());
    // URLGetter.download receives its own headers, rather than the environment's.
    request.remove("request_headers");
    request.remove("curl_opts");
    if let Some(headers) = headers {
        request.insert("request_headers".into(), headers.into());
    }
    super::download::fetch(&request)
}
fn format_url(template: &str, fields: &[(&str, &str)]) -> Result<String> {
    let mut result = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                result.push('{');
                continue;
            }
            let mut key = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '}' {
                    closed = true;
                    break;
                }
                key.push(c);
            }
            if !closed {
                return Err("Single '{' encountered in format string".into());
            }
            result.push_str(
                fields
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| *v)
                    .ok_or_else(|| format!("Unknown URL template field '{key}'"))?,
            );
        } else if c == '}' {
            if chars.next() != Some('}') {
                return Err("Single '}' encountered in format string".into());
            }
            result.push('}');
        } else {
            result.push(c);
        }
    }
    Ok(result)
}
fn product_release(product: &str, release: &str) -> String {
    match release {
        "latest-esr" | "esr-latest" => format!("{product}-esr-latest"),
        "latest-beta" | "beta-latest" => format!("{product}-beta-latest"),
        _ => format!("{product}-{release}"),
    }
}
fn normalize(version: &str) -> String {
    version
        .replace(['a', 'b'], ".0.")
        .replace("esr", "")
        .replace("-msi", "")
}
fn mozilla(env: &mut Dictionary) -> Result<()> {
    let product = string(env, "product_name")?.to_owned();
    let release = get(env, "release", "latest").to_owned();
    let locale = get(env, "locale", "en-US").replace('_', "-");
    let pr = product_release(&product, &release);
    let url = format_url(
        get(env, "base_url", MOZ_URL),
        &[
            ("product_release", &pr),
            ("platform", get(env, "platform", "osx")),
            ("locale", &locale),
        ],
    )?;
    env.insert("url".into(), url.clone().into());
    let simple = if product.contains("firefox") {
        "firefox"
    } else if product.contains("thunderbird") {
        "thunderbird"
    } else {
        return Err(format!("Product '{product}' is not a supported product."));
    };
    let upper = simple.to_uppercase();
    let key = if pr.contains("esr") {
        Some(format!("{upper}_ESR"))
    } else if pr.contains("beta") {
        Some(format!("LATEST_{upper}_DEVEL_VERSION"))
    } else if pr.contains("nightly") {
        Some(format!("{upper}_NIGHTLY"))
    } else if pr.contains("latest") {
        Some(format!("LATEST_{upper}_VERSION"))
    } else {
        None
    };
    let original = if let Some(key) = key {
        let endpoint = format_url(
            get(env, "versions_base_url", MOZ_VERSIONS),
            &[("product", simple)],
        )?;
        let data: serde_json::Value =
            serde_json::from_str(&fetch(env, &endpoint, None)?).map_err(|e| e.to_string())?;
        data.get(&key)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("Missing release version '{key}'"))?
            .to_owned()
    } else {
        release
    };
    env.insert("moz_version".into(), normalize(&original).into());
    env.insert("moz_original_version".into(), original.into());
    env.insert("moz_locale".into(), locale.into());
    output(format!("Found URL {url}"));
    Ok(())
}
fn barebones_metadata(env: &mut Dictionary, data: &str) -> Result<()> {
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
fn edge_metadata(
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
fn office_metadata(
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
pub(super) fn office_is_deprecated(env: &Dictionary) -> bool {
    matches!(
        env.get("product").and_then(Value::as_string),
        Some("Excel2016" | "OneNote2016" | "Outlook2016" | "PowerPoint2016" | "Word2016")
    )
}
fn office(env: &mut Dictionary) -> Result<()> {
    let mode = get(env, "version", "latest");
    if !["latest", "latest-delta", "latest-standalone"].contains(&mode) {
        return Err(
            "Invalid 'version': supported values are 'latest', 'latest-delta', 'latest-standalone'"
                .into(),
        );
    }
    let product = string(env, "product")?.to_owned();
    if office_is_deprecated(env) {
        super::warning(env, format!("As of August 2026, Microsoft no longer publishes {product} updates to the Microsoft AutoUpdate feed. Office for Mac 2016 support ended in October 2020 at version 16.16.27. Please use the MS{} recipes instead.",product.replace("2016","2019")))?;
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
pub(super) fn execute(
    name: &str,
    env: &mut Dictionary,
    _preferences: Option<&Dictionary>,
) -> Result<()> {
    match name {
        "MozillaURLProvider" => mozilla(env),
        "MSOfficeMacURLandUpdateInfoProvider" => office(env),
        "BarebonesURLProvider" => {
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
        _ => Err(format!("Unknown community provider {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xml(value: Dictionary) -> String {
        let mut bytes = Vec::new();
        Value::Dictionary(value).to_writer_xml(&mut bytes).unwrap();
        String::from_utf8(bytes).unwrap()
    }
    #[test]
    fn mozilla_aliases_versions_and_templates() {
        for (input, expected) in [
            ("79.0b9", "79.0.0.9"),
            ("68.10.0esr", "68.10.0"),
            ("77.0-msi", "77.0"),
            ("80.0a1", "80.0.0.1"),
        ] {
            assert_eq!(normalize(input), expected);
        }
        assert_eq!(
            product_release("firefox", "latest-esr"),
            "firefox-esr-latest"
        );
        assert_eq!(
            format_url("{{x}}/{product}", &[("product", "firefox")]).unwrap(),
            "{x}/firefox"
        );
        assert!(format_url("{missing}", &[]).is_err());
        let mut env = Dictionary::from_iter([
            ("product_name", "firefox"),
            ("release", "77.0-msi"),
            ("locale", "en_US"),
        ]);
        mozilla(&mut env).unwrap();
        assert_eq!(env["moz_version"].as_string(), Some("77.0"));
        assert_eq!(env["moz_locale"].as_string(), Some("en-US"));
        assert_eq!(
            env["url"].as_string(),
            Some("https://download.mozilla.org/?product=firefox-77.0-msi-ssl&os=osx&lang=en-US")
        );
    }
    #[test]
    fn mozilla_http_release_lookup() {
        let body = r#"{"LATEST_FIREFOX_DEVEL_VERSION":"151.0b9","LATEST_FIREFOX_VERSION":"150.0"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (url, server) = crate::downloader::tests::server(vec![response]);
        let mut env = Dictionary::from_iter([
            ("product_name", "firefox"),
            ("release", "beta-latest"),
            ("versions_base_url", url.as_str()),
        ]);
        mozilla(&mut env).unwrap();
        assert_eq!(env["moz_original_version"].as_string(), Some("151.0b9"));
        assert_eq!(env["moz_version"].as_string(), Some("151.0.0.9"));
        server.join().unwrap();
    }
    #[test]
    fn barebones_sorts_numerically_and_selects_last_equal() {
        let entries = [("9.0", "old"), ("10.0", "first"), ("10.0.0", "last")]
            .into_iter()
            .map(|(version, url)| {
                Value::Dictionary(Dictionary::from_iter([
                    ("SUFeedEntryShortVersionString", version),
                    ("SUFeedEntryDownloadURL", url),
                    ("SUFeedEntryMinimumSystemVersion", "12.0"),
                ]))
            })
            .collect();
        let mut manifest = Dictionary::new();
        manifest.insert("SUFeedEntries".into(), Value::Array(entries));
        let mut env = Dictionary::new();
        barebones_metadata(&mut env, &xml(manifest)).unwrap();
        assert_eq!(env["url"].as_string(), Some("last"));
        assert!(barebones_metadata(&mut env, &xml(Dictionary::new()))
            .unwrap_err()
            .contains("SUFeedEntries"));
        assert!(barebones_metadata(&mut env, "not plist")
            .unwrap_err()
            .contains("Unexpected error parsing manifest"));
    }
    #[test]
    fn edge_filters_platform_architecture_artifact_and_orders_versions() {
        let data = r#"[{"Product":"Stable","Releases":[{"Platform":"windows","Architecture":"universal","ProductVersion":"999","Artifacts":[{"ArtifactName":"pkg","Location":"wrong"}]},{"Platform":"macos","Architecture":"arm64","ProductVersion":"999","Artifacts":[{"ArtifactName":"pkg","Location":"wrong"}]},{"Platform":"MacOS","Architecture":"Universal","ProductVersion":"9.0","Artifacts":[{"ArtifactName":"pkg","Location":"old"}]},{"Platform":"macos","Architecture":"universal","ProductVersion":"10.0","Artifacts":[{"ArtifactName":"zip","Location":"ignored"},{"ArtifactName":"pkg","Location":" https://example.test/latest.pkg\n"}]}]}]"#;
        let mut env = Dictionary::new();
        edge_metadata(
            &mut env,
            data,
            "Stable",
            "com.microsoft.edgemac",
            "/Applications/Microsoft Edge.app",
        )
        .unwrap();
        assert_eq!(env["version"].as_string(), Some("10.0"));
        assert_eq!(
            env["url"].as_string(),
            Some("https://example.test/latest.pkg")
        );
        assert!(!env.contains_key("minimum_os_version"));
        assert!(edge_metadata(&mut env, data, "Beta", "id", "path")
            .unwrap_err()
            .contains("Could not find Edge channel"));
        assert!(edge_metadata(
            &mut env,
            r#"[{"Product":"Stable","Releases":[]}]"#,
            "Stable",
            "id",
            "path"
        )
        .unwrap_err()
        .contains("universal pkg"));
    }
    fn office_feed(delta: bool) -> String {
        let mut item = Dictionary::from_iter([
            ("Location", " https://example.test/App_Updater.pkg\n"),
            ("Title", " App Update "),
            ("Update Version", "16.99"),
            ("Minimum OS", "10.1"),
        ]);
        item.insert(
            "Trigger Condition".into(),
            Value::Array(vec!["and".into(), "Registered File".into()]),
        );
        if delta {
            item.insert("FullUpdaterLocation".into(), "full".into());
            let mut registered = Dictionary::new();
            registered.insert(
                "VersionsRelative".into(),
                Value::Array(vec!["> 1".into(), ">= 16.90".into()]),
            );
            let mut triggers = Dictionary::new();
            triggers.insert("Registered File".into(), registered.into());
            item.insert("Triggers".into(), triggers.into());
        }
        let mut bytes = Vec::new();
        Value::Array(vec![item.into()])
            .to_writer_xml(&mut bytes)
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }
    #[test]
    fn office_standalone_delta_and_minimum_os() {
        let products: serde_json::Value = serde_json::from_str(PRODUCTS).unwrap();
        let spec = &products["Excel2019"];
        let mut env = Dictionary::from_iter([("version", "latest-standalone")]);
        office_metadata(&mut env, &office_feed(false), "Excel2019", spec, "endpoint").unwrap();
        assert_eq!(
            env["url"].as_string(),
            Some("https://example.test/App_Installer.pkg")
        );
        assert_eq!(env["minimum_os_version"].as_string(), Some("10.12"));
        let install = &env["additional_pkginfo"].as_dictionary().unwrap()["installs"]
            .as_array()
            .unwrap()[0];
        assert_eq!(
            install.as_dictionary().unwrap()["minimum_update_version"].as_string(),
            Some("16.17")
        );
        let mut env = Dictionary::from_iter([
            ("version", "latest-delta"),
            ("NAME", "Excel"),
            ("munki_required_update_name", "Custom"),
        ]);
        office_metadata(&mut env, &office_feed(true), "Excel2019", spec, "endpoint").unwrap();
        assert_eq!(env["minimum_version_for_delta"].as_string(), Some("16.90"));
        assert_eq!(
            env["additional_pkginfo"].as_dictionary().unwrap()["requires"]
                .as_array()
                .unwrap()[0]
                .as_string(),
            Some("Custom-16.90")
        );
        let mut env = Dictionary::from_iter([("version", "latest-delta")]);
        assert!(
            office_metadata(&mut env, &office_feed(false), "Excel2019", spec, "endpoint")
                .unwrap_err()
                .contains("applicable update")
        );
    }
    #[test]
    fn office_rejections_and_deprecation_do_not_fetch() {
        let mut env = Dictionary::from_iter([("version", "old"), ("product", "Edge")]);
        assert!(office(&mut env).unwrap_err().contains("Invalid 'version'"));
        env.insert("version".into(), "latest-delta".into());
        assert!(office(&mut env).unwrap_err().contains("Edge supports only"));
        env.insert("version".into(), "latest".into());
        env.insert("channel".into(), "invalid".into());
        assert!(office(&mut env).unwrap_err().contains("Edge CHANNEL"));
        env.insert("product".into(), "Excel2019".into());
        assert!(office(&mut env).unwrap_err().contains("custom uuid"));
        env.insert("product".into(), "Excel2016".into());
        env.insert("RECIPE_PATH".into(), "/tmp/Excel.download.recipe".into());
        office(&mut env).unwrap();
        assert_eq!(env["stop_processing_recipe"].as_boolean(), Some(true));
        assert!(env.contains_key("deprecation_summary_result"));
    }
}
