//! `PuppetlabsProductsURLProvider`: find the download URL for a Puppet Labs
//! product. A native port of the autopkg/recipes processor (Apache-2.0).
//!
//! Inputs and outputs: run `russet processor-info PuppetlabsProductsURLProvider`, or see
//! `PuppetlabsProductsURLProvider` in `compatibility/community-processors.json`.
use crate::community_legacy::{fetch, get, output, Result};
use plist::Dictionary;

pub(crate) fn puppet_candidate(env: &Dictionary, data: &str) -> Result<(String, String)> {
    let product = crate::string(env, "product_name")?;
    let pattern = if product == "agent" {
        format!(
            r#"href="(puppet-agent-(\d+\.\d+\.\d+)-1.osx({}).dmg)""#,
            get(env, "get_os_version", "10.10")
        )
    } else {
        let version = get(env, "get_version", "latest");
        let version = if version.is_empty() || version == "latest" {
            r"\d+[\.\d]+"
        } else {
            version
        };
        format!(r#"href="({}-({version})+.dmg)""#, product.to_lowercase())
    };
    let re = fancy_regex::Regex::new(&pattern).map_err(|e| e.to_string())?;
    let mut highest: Option<(String, String)> = None;
    for candidate in re.captures_iter(data) {
        let candidate = candidate.map_err(|e| e.to_string())?;
        let item = (candidate[1].to_owned(), candidate[2].to_owned());
        if highest
            .as_ref()
            .is_none_or(|old| autopkg_platform::github::compare_versions(&item.1, &old.1).is_gt())
        {
            highest = Some(item);
        }
    }
    highest.ok_or_else(|| "Unable to parse any products from download index.".into())
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    let url = if crate::string(env, "product_name")? == "agent" {
        format!(
            "https://downloads.puppetlabs.com/mac/{}/PC1/x86_64",
            get(env, "get_os_version", "10.10")
        )
    } else {
        "https://downloads.puppetlabs.com/mac".into()
    };
    let (file, version) = puppet_candidate(env, &fetch(env, &url)?)?;
    env.insert("version".into(), version.into());
    env.insert("url".into(), format!("{url}/{file}").into());
    output(1, format!("Found URL {url}/{file}"));
    Ok(())
}
