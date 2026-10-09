//! `AdobeReaderURLProvider`: find the download URL for Adobe Reader. A native
//! port of the autopkg/recipes processor (Apache-2.0).
use crate::community_legacy::{fetch, get, output, Result, TypedResult};
use plist::{Dictionary, Value};

fn json_log(data: &str) -> TypedResult<()> {
    // Deserialize directly into the ordered dictionary so debug output keeps
    // the server's key order, matching Python's json.loads representation.
    let value: Value = serde_json::from_str(data)
        .map_err(|error| crate::ExecutionFailure::unexpected(error.to_string()))?;
    output(3, plist::python_repr(&value));
    Ok(())
}
fn quoted(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"_.-~/".contains(b) {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn json_string<'a>(v: &'a serde_json::Value, key: &str) -> TypedResult<&'a str> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            crate::ExecutionFailure::unexpected(format!(
                "Missing or invalid {key} in Adobe response"
            ))
        })
}
pub(crate) fn execute_typed(env: &mut Dictionary) -> TypedResult<()> {
    let os = get(env, "os_version", "Mac OS 10.14.0");
    if !os.starts_with("Mac OS") {
        output(1, format!("WARNING: Please update the OS_VERSION in your override from '{os}' to 'Mac OS {os}'"));
    }
    let os = quoted(&if os.starts_with("Mac OS") {
        os.into()
    } else {
        format!("Mac OS {os}")
    });
    let products = get(
        env,
        "base_url",
        "https://rdc.adobe.io/reader/products?os={OS_VERSION}&api_key=dc-get-adobereader-cdn",
    )
    .replace("{OS_VERSION}", &os);
    output(3, format!("RDC_PRODUCTS_URL: {products}"));
    let body = fetch(env, &products)?;
    let data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| crate::ExecutionFailure::unexpected(e.to_string()))?;
    json_log(&body)?;
    let product = data.pointer("/products/reader/0").ok_or_else(|| {
        crate::ExecutionFailure::unexpected("Missing reader product in Adobe response")
    })?;
    output(
        1,
        format!("[displayName] : {}", json_string(product, "displayName")?),
    );
    output(
        1,
        format!("[version]     : {}", json_string(product, "version")?),
    );
    let name = quoted(json_string(product, "displayName")?);
    env.insert("version".into(), json_string(product, "version")?.into());
    let url = get(env,"download_url","https://rdc.adobe.io/reader/downloadUrl?name={DISPLAY_NAME}&os={OS_VERSION}&api_key=dc-get-adobereader-cdn").replace("{OS_VERSION}",&os).replace("{DISPLAY_NAME}",&name);
    output(3, format!("RDC_DOWNLOAD_URL: {url}"));
    let body = fetch(env, &url)?;
    let data: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| crate::ExecutionFailure::unexpected(e.to_string()))?;
    json_log(&body)?;
    output(
        1,
        format!("[download_url]: {}", json_string(&data, "downloadURL")?),
    );
    env.insert("url".into(), json_string(&data, "downloadURL")?.into());
    env.insert("filename".into(), json_string(&data, "saveName")?.into());
    Ok(())
}

pub(crate) fn execute(env: &mut Dictionary) -> Result<()> {
    execute_typed(env).map_err(|e| e.message)
}
