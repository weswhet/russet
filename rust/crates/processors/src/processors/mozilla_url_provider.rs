//! `MozillaURLProvider`: find the download URL for a Mozilla product. A
//! native port of the autopkg/recipes processor (Apache-2.0).
use crate::community_modern::{fetch, get, output};
use crate::{string, Result};
use plist::Dictionary;

const MOZ_URL: &str =
    "https://download.mozilla.org/?product={product_release}-ssl&os={platform}&lang={locale}";
const MOZ_VERSIONS: &str = "https://product-details.mozilla.org/1.0/{product}_versions.json";
pub(crate) fn format_url(template: &str, fields: &[(&str, &str)]) -> Result<String> {
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
pub(crate) fn product_release(product: &str, release: &str) -> String {
    match release {
        "latest-esr" | "esr-latest" => format!("{product}-esr-latest"),
        "latest-beta" | "beta-latest" => format!("{product}-beta-latest"),
        _ => format!("{product}-{release}"),
    }
}
pub(crate) fn normalize(version: &str) -> String {
    version
        .replace(['a', 'b'], ".0.")
        .replace("esr", "")
        .replace("-msi", "")
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
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
