//! `AdobeFlashURLProvider`: find the download URL for Adobe Flash Player. A
//! native port of the autopkg/recipes processor (Apache-2.0).
use crate::community_legacy::{fetch_common, get, output, Result};
use plist::Dictionary;

pub(crate) fn flash_version(xml: &str) -> Result<String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("Can't read {xml}: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "XML" {
        return Err("Update XML in unexpected format.".into());
    }
    root.children()
        .find(|n| n.has_tag_name("update"))
        .and_then(|n| n.attribute("version"))
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Update XML in unexpected format.".into())
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    if let Some(url) = env.get("url") {
        output(1, format!("Using input URL {}", plist::python_str(url)));
        return Ok(());
    }
    let version = get(env, "version", "");
    let version = if version.is_empty() {
        flash_version(&fetch_common(env,"http://fpdownload2.macromedia.com/get/flashplayer/update/current/xml/version_en_mac_pl.xml")?)?
    } else {
        output(1, format!("Using provided version {version}"));
        version.into()
    };
    env.insert(
        "url".into(),
        format!(
            "https://fpdownload.macromedia.com/get/flashplayer/pdc/{}/install_flash_player_osx.dmg",
            version.replace(',', ".")
        )
        .into(),
    );
    output(1, format!("Found URL {}", crate::string(env, "url")?));
    Ok(())
}
