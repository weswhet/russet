use super::{string, truth, Result};
use plist::{Dictionary, Value};
use std::{cmp::Ordering, collections::BTreeMap};
const NAMESPACE: &str = "http://www.andymatuschak.org/xml-namespaces/sparkle";
type Item = BTreeMap<String, String>;
fn quote(path: &str) -> String {
    let mut output = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}
fn build_url(url: &str, encode: bool) -> Result<String> {
    let url = url.split('#').next().unwrap();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or("Sparkle enclosure URL must be absolute")?;
    let (authority, path) = rest
        .split_once('/')
        .map(|(a, p)| (a, format!("/{p}")))
        .unwrap_or((rest, String::new()));
    let (path, query) = path
        .split_once('?')
        .map(|(p, q)| (p, Some(q)))
        .unwrap_or((&path, None));
    let mut url = format!(
        "{scheme}://{authority}{}",
        if encode { quote(path) } else { path.into() }
    );
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    Ok(url)
}
fn parts(version: &str) -> Vec<String> {
    let regex = regex::Regex::new(r"(\d+|[a-z]+|\.)").unwrap();
    let mut parts = Vec::new();
    let mut pos = 0;
    for token in regex.find_iter(version) {
        if token.start() > pos {
            parts.push(version[pos..token.start()].into());
        }
        if token.as_str() != "." {
            parts.push(token.as_str().into());
        }
        pos = token.end();
    }
    if pos < version.len() {
        parts.push(version[pos..].into());
    }
    parts
}
pub(super) fn version_cmp(a: &str, b: &str) -> Ordering {
    let a = parts(a);
    let b = parts(b);
    for i in 0..a.len().max(b.len()) {
        let a = a.get(i).map(String::as_str).unwrap_or("0");
        let b = b.get(i).map(String::as_str).unwrap_or("0");
        let an = a.chars().all(|c| c.is_ascii_digit());
        let bn = b.chars().all(|c| c.is_ascii_digit());
        let order = match (an, bn) {
            (true, true) => {
                let a = a.trim_start_matches('0');
                let b = b.trim_start_matches('0');
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}
fn parse(data: &str, namespace: &str, encode: bool) -> Result<Vec<Item>> {
    let document =
        roxmltree::Document::parse(data).map_err(|_| "Error parsing XML from appcast feed.")?;
    let root = document.root_element();
    let items: Vec<_> = root
        .children()
        .filter(|n| {
            n.is_element() && n.tag_name().name() == "channel" && n.tag_name().namespace().is_none()
        })
        .flat_map(|n| n.children())
        .filter(|n| {
            n.is_element() && n.tag_name().name() == "item" && n.tag_name().namespace().is_none()
        })
        .collect();
    if items.is_empty() {
        return Err("No channel items were found in appcast feed.".into());
    }
    let mut versions = Vec::new();
    for node in items {
        let Some(enclosure) = node.children().find(|n| {
            n.is_element()
                && n.tag_name().name() == "enclosure"
                && n.tag_name().namespace().is_none()
        }) else {
            continue;
        };
        let Some(url) = enclosure.attribute("url").filter(|s| !s.is_empty()) else {
            continue;
        };
        let url = build_url(url, encode)?;
        let child = |name: &str| {
            node.children()
                .find(|n| {
                    n.is_element()
                        && n.tag_name().name() == name
                        && n.tag_name().namespace() == Some(namespace)
                })
                .and_then(|n| n.text())
                .map(str::trim)
        };
        let version = child("version")
            .or_else(|| enclosure.attribute((namespace, "version")))
            .map(str::to_string)
            .or_else(|| {
                let filename = url.rsplit('/').next()?;
                let filename = filename
                    .rsplit_once('.')
                    .map(|(a, _)| a)
                    .unwrap_or(filename);
                ['_', '-'].into_iter().find_map(|delimiter| {
                    filename
                        .contains(delimiter)
                        .then(|| filename.rsplit(delimiter).next().unwrap().into())
                })
            })
            .ok_or("Can't extract version info from item in feed!")?;
        let mut item = Item::new();
        item.insert("url".into(), url.trim().into());
        item.insert("version".into(), version.trim().into());
        if let Some(version) = child("shortVersionString")
            .or_else(|| enclosure.attribute((namespace, "shortVersionString")))
        {
            item.insert("human_version".into(), version.trim().into());
        }
        for (tag, key) in [
            ("minimumSystemVersion", "minimum_os_version"),
            ("channel", "channel"),
            ("releaseNotesLink", "description_url"),
        ] {
            if let Some(text) = child(tag) {
                item.insert(key.into(), text.into());
            }
        }
        if let Some(text) = node
            .children()
            .find(|n| {
                n.is_element()
                    && n.tag_name().name() == "description"
                    && n.tag_name().namespace().is_none()
            })
            .and_then(|n| n.text())
        {
            item.insert("description_data".into(), text.trim().into());
        }
        versions.push(item);
    }
    Ok(versions)
}
fn fetch(env: &Dictionary, url: &str, extra: Option<&Dictionary>) -> Result<String> {
    let mut request = env.clone();
    request.insert("url".into(), url.into());
    if let Some(extra) = extra {
        let mut headers = env
            .get("request_headers")
            .and_then(Value::as_dictionary)
            .cloned()
            .unwrap_or_default();
        for (k, v) in extra {
            headers.insert(k.clone(), v.clone());
        }
        request.insert("request_headers".into(), headers.into());
    }
    super::download::fetch(&request)
}
fn description_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value).map_err(|_| {
        "Sparkle feed description URL must be an http(s) URL with a non-loopback hostname."
    })?;
    if !["http", "https"].contains(&url.scheme()) || url.host_str().is_none() {
        return Err(
            "Sparkle feed description URL must be an http(s) URL with a non-loopback hostname."
                .into(),
        );
    }
    let host = url
        .host_str()
        .unwrap()
        .trim_end_matches('.')
        .trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") {
        return Err("Sparkle feed description URL cannot use localhost.".into());
    }
    if host
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
    {
        return Err("Sparkle feed description URL cannot use a loopback address.".into());
    }
    Ok(())
}
pub(super) fn execute(env: &mut Dictionary) -> Result<()> {
    if let Some(pkg) = env.get("PKG") {
        autopkg_platform::processor_output(1, "Local PKG provided, no downloaded needed.");
        autopkg_platform::processor_output(1, "WARNING: Skipping this processor means output variables 'version', 'additional_pkginfo' will not contain useful info. If these are needed in other recipe steps, this may give unexpected results.");
        env.insert("url".into(), pkg.clone());
        env.insert("version".into(), "NotSetBySparkleUpdateInfoProvider".into());
        env.insert("additional_pkginfo".into(), Dictionary::new().into());
        return Ok(());
    }
    let mut appcast = string(env, "appcast_url")?.to_string();
    if let Some(queries) = env.get("appcast_query_pairs") {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in queries
            .as_dictionary()
            .ok_or("appcast_query_pairs must be a dictionary")?
        {
            let text = match v {
                Value::String(s) => s.clone(),
                Value::Integer(n) => n.to_string(),
                Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
                _ => return Err("appcast_query_pairs values must be scalar values".into()),
            };
            query.append_pair(k, &text);
        }
        let mut url = url::Url::parse(&appcast).map_err(|e| e.to_string())?;
        url.set_query(Some(&query.finish()));
        appcast = url.into();
    }
    let extra = env
        .get("appcast_request_headers")
        .map(|v| {
            v.as_dictionary()
                .ok_or("appcast_request_headers must be a dictionary")
        })
        .transpose()?;
    let data = fetch(env, &appcast, extra)?;
    let namespace = env
        .get("alternate_xmlns_url")
        .and_then(Value::as_string)
        .unwrap_or(NAMESPACE);
    let items = parse(&data, namespace, truth(env.get("urlencode_path_component")))?;
    autopkg_platform::processor_output(1, format!("Items in feed: {}", items.len()));
    let channel = env
        .get("update_channel")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty());
    let mut eligible = items.iter().filter(|item| {
        item.get("channel")
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            == channel
    });
    autopkg_platform::processor_output(
        1,
        format!(
            "Items in {} channel: {}",
            channel.unwrap_or("default"),
            eligible.clone().count()
        ),
    );
    let mut latest = eligible.next().ok_or_else(|| {
        format!(
            "No items were found in {} channel.",
            channel.unwrap_or("default")
        )
    })?;
    for item in eligible {
        if version_cmp(&item["version"], &latest["version"]) == Ordering::Greater {
            latest = item;
        }
    }
    autopkg_platform::processor_output(
        1,
        format!("Version retrieved from appcast: {}", latest["version"]),
    );
    if let Some(version) = latest.get("human_version").filter(|s| !s.is_empty()) {
        autopkg_platform::processor_output(
            1,
            format!("User-facing version retrieved from appcast: {version}"),
        );
    }
    let keys = env
        .get("pkginfo_keys_to_copy_from_sparkle_feed")
        .map(|v| {
            v.as_array()
                .ok_or("pkginfo_keys_to_copy_from_sparkle_feed must be an array")
        })
        .transpose()?;
    let mut pkginfo = Dictionary::new();
    if let Some(keys) = keys {
        for key in keys {
            match key.as_string().ok_or("pkginfo keys must be strings")? {
                "description" => {
                    let text = if let Some(url) = latest.get("description_url") {
                        description_url(url)?;
                        fetch(env, url, None)?
                    } else if let Some(data) = latest.get("description_data") {
                        format!("<html><body>{data}</body></html>")
                    } else {
                        String::new()
                    };
                    pkginfo.insert("description".into(), text.into());
                }
                "minimum_os_version" => {
                    if let Some(version) = latest.get("minimum_os_version") {
                        pkginfo.insert("minimum_os_version".into(), version.clone().into());
                    }
                }
                key => autopkg_platform::processor_output(1, format!("Key {key} isn't a supported key to copy from the Sparkle feed, ignoring it.")),
            }
        }
    }
    for key in pkginfo.keys() {
        autopkg_platform::processor_output(
            1,
            format!("Copied key {key} from Sparkle feed to additional pkginfo."),
        );
    }
    autopkg_platform::processor_output(1, format!("Found URL {}", latest["url"]));
    env.insert("url".into(), latest["url"].clone().into());
    env.insert(
        "version".into(),
        latest
            .get("human_version")
            .unwrap_or(&latest["version"])
            .clone()
            .into(),
    );
    env.insert("additional_pkginfo".into(), pkginfo.into());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn namespaced_items_and_version_order() {
        let xml = r#"<rss xmlns:s="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item><enclosure url="https://example.com/App 1%20.zip" s:version="9" s:shortVersionString="1.9"/></item><item><s:version>10</s:version><s:shortVersionString>2.0</s:shortVersionString><s:channel>beta</s:channel><enclosure url="https://example.com/app.zip"/></item></channel></rss>"#;
        let items = parse(xml, NAMESPACE, true).unwrap();
        assert_eq!(items[0]["url"], "https://example.com/App%201%2520.zip");
        assert_eq!(items[1]["human_version"], "2.0");
        assert_eq!(version_cmp("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_cmp("10", "9"), Ordering::Greater);
        assert_eq!(version_cmp("1.0b1", "1.0"), Ordering::Greater);
    }
    #[test]
    fn rejects_description_loopback() {
        for value in [
            "file:///tmp/notes",
            "http://localhost./",
            "http://127.0.0.1/",
            "http://[::1]/",
        ] {
            assert!(description_url(value).is_err());
        }
        assert!(description_url("https://example.com/notes").is_ok());
    }
    #[test]
    fn ignores_wrong_namespace_and_derives_version() {
        let xml = r#"<rss xmlns:s="https://wrong.example"><channel><item><s:version>999</s:version><enclosure url="https://example.com/App_2.1.zip"/></item></channel></rss>"#;
        assert_eq!(parse(xml, NAMESPACE, true).unwrap()[0]["version"], "2.1");
    }
}

#[cfg(test)]
#[test]
fn appcast_http_query_channel_and_metadata() {
    let body = r#"<rss xmlns:s="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item><enclosure url="https://example.com/stable.zip" s:version="100"/></item><item><s:channel>beta</s:channel><s:minimumSystemVersion>13.0</s:minimumSystemVersion><description><![CDATA[<p>Changes</p>]]></description><enclosure url="https://example.com/beta.zip" s:version="12" s:shortVersionString="2.0b1"/></item></channel></rss>"#;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (url, server) = crate::downloader::tests::server(vec![response]);
    let mut env = crate::tests::env(&[("appcast_url", &url), ("update_channel", "beta")]);
    let mut query = Dictionary::new();
    query.insert("channel name".into(), "beta test".into());
    env.insert("appcast_query_pairs".into(), query.into());
    env.insert(
        "pkginfo_keys_to_copy_from_sparkle_feed".into(),
        Value::Array(vec!["minimum_os_version".into(), "description".into()]),
    );
    crate::execute("SparkleUpdateInfoProvider", &mut env).unwrap();
    assert_eq!(env["version"].as_string(), Some("2.0b1"));
    assert_eq!(env["url"].as_string(), Some("https://example.com/beta.zip"));
    let metadata = env["additional_pkginfo"].as_dictionary().unwrap();
    assert_eq!(metadata["minimum_os_version"].as_string(), Some("13.0"));
    assert_eq!(
        metadata["description"].as_string(),
        Some("<html><body><p>Changes</p></body></html>")
    );
    assert!(server.join().unwrap()[0].contains("?channel+name=beta+test"));
}
