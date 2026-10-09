//! Native ports of the Apache-2.0 AutoPkg community providers by Allister Banks,
//! Tim Sutton, Greg Neagle, Carl Ashley, and Per Olofsson.
use super::Result;
use plist::{Dictionary, Value};
pub(crate) fn output(message: impl AsRef<str>) {
    autopkg_platform::processor_output(1, message.as_ref());
}
pub(crate) fn get<'a>(env: &'a Dictionary, key: &str, default: &'a str) -> &'a str {
    env.get(key).and_then(Value::as_string).unwrap_or(default)
}
pub(crate) fn fetch(env: &Dictionary, url: &str, headers: Option<Dictionary>) -> Result<String> {
    let mut request = env.clone();
    request.insert("url".into(), url.into());
    // URLGetter.download receives its own headers, rather than the environment's.
    request.remove("request_headers");
    request.remove("curl_opts");
    if let Some(headers) = headers {
        request.insert("request_headers".into(), headers.into());
    }
    crate::processors::url_getter::fetch(&request)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::processors::{
        barebones_url_provider::barebones_metadata,
        mozilla_url_provider::{execute as mozilla, format_url, normalize, product_release},
        ms_office_mac_url_and_update_info_provider::{
            edge_metadata, execute as office, office_metadata, PRODUCTS,
        },
    };
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
        let (url, server) = crate::processors::url_downloader::tests::server(vec![response]);
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
