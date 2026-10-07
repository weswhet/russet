//! Native ports of the pinned autopkg/recipes Adobe, Puppet and Sassafras processors.
//! Original processors: Timothy Sutton, Per Olofsson, Glynn Lane, Nate Felton,
//! Greg Neagle and Allister Banks, Apache-2.0.
use plist::{Dictionary, Value};
use std::{fs, path::Path, process::Command};
type Result<T> = std::result::Result<T, String>;
type TypedResult<T> = std::result::Result<T, super::ExecutionFailure>;
fn output(level: i64, message: impl std::fmt::Display) {
    autopkg_platform::processor_output(level, message);
}
fn json_log(data: &str) -> TypedResult<()> {
    // Deserialize directly into the ordered dictionary so debug output keeps
    // the server's key order, matching Python's json.loads representation.
    let value: Value = serde_json::from_str(data)
        .map_err(|error| super::ExecutionFailure::unexpected(error.to_string()))?;
    output(3, plist::python_repr(&value));
    Ok(())
}
fn get<'a>(env: &'a Dictionary, key: &str, default: &'a str) -> &'a str {
    env.get(key).and_then(Value::as_string).unwrap_or(default)
}
fn fetch(env: &Dictionary, url: &str) -> Result<String> {
    let mut request = env.clone();
    // URLGetter.download passes only its explicit headers. Flash alone calls
    // add_curl_common_opts and therefore inherits recipe headers/options.
    request.remove("request_headers");
    request.remove("curl_opts");
    fetch_common(&request, url)
}
fn fetch_common(env: &Dictionary, url: &str) -> Result<String> {
    let mut request = env.clone();
    request.insert("url".into(), url.into());
    super::download::fetch(&request)
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
            super::ExecutionFailure::unexpected(format!(
                "Missing or invalid {key} in Adobe response"
            ))
        })
}
fn reader(env: &mut Dictionary) -> TypedResult<()> {
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
        .map_err(|e| super::ExecutionFailure::unexpected(e.to_string()))?;
    json_log(&body)?;
    let product = data.pointer("/products/reader/0").ok_or_else(|| {
        super::ExecutionFailure::unexpected("Missing reader product in Adobe response")
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
        .map_err(|e| super::ExecutionFailure::unexpected(e.to_string()))?;
    json_log(&body)?;
    output(
        1,
        format!("[download_url]: {}", json_string(&data, "downloadURL")?),
    );
    env.insert("url".into(), json_string(&data, "downloadURL")?.into());
    env.insert("filename".into(), json_string(&data, "saveName")?.into());
    Ok(())
}
fn flash_version(xml: &str) -> Result<String> {
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
fn flash(env: &mut Dictionary) -> Result<()> {
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
    output(1, format!("Found URL {}", super::string(env, "url")?));
    Ok(())
}
fn puppet_candidate(env: &Dictionary, data: &str) -> Result<(String, String)> {
    let product = super::string(env, "product_name")?;
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
fn puppet(env: &mut Dictionary) -> Result<()> {
    let url = if super::string(env, "product_name")? == "agent" {
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
fn acrobat<F>(env: &mut Dictionary, mut fetcher: F) -> TypedResult<()>
where
    F: FnMut(&Dictionary, &str) -> Result<String>,
{
    let major = super::string(env, "major_version")?.to_owned();
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
    let previous = super::string(&data, "PreviousURLTemplate")?;
    if previous == "noTemplate" {
        return Err(super::ExecutionFailure::unexpected(
            "local variable 'prev_version' referenced before assignment",
        ));
    }
    let previous = manifest(&substitute(&format!("{base}{previous}")))?;
    let previous = super::string(&previous, "BuildNumber")?;
    let version = super::string(&data, "BuildNumber")?;
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
            super::string(&data, "PatchURL")?
        )
        .into(),
    );
    output(1, format!("Found URL {}", super::string(env, "url")?));
    Ok(())
}
fn modify_distribution(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err("%s not found".into());
    }
    let bytes = fs::read(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    let mut root = xmltree::Element::parse(bytes.as_slice())
        .map_err(|e| format!("Can't read {}: {e}", path.display()))?;
    if !["installer-script", "installer-gui-script"].contains(&root.name.as_str()) {
        return Err("Distribution file is not in the expected format.".into());
    }
    if let Some(index) = root
        .children
        .iter()
        .position(|n| n.as_element().is_some_and(|e| e.name == "domains"))
    {
        root.children.remove(index);
        let output = fs::File::create(path).map_err(|e| e.to_string())?;
        root.write_with_config(
            output,
            xmltree::EmitterConfig::new().write_document_declaration(false),
        )
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    }
    Ok(())
}
fn replace_preinstall(expanded: &Path) -> Result<()> {
    let app = expanded.join("application_mini_7z.pkg");
    if !app.exists() {
        return Err("application_mini_7z.pkg not found!".into());
    }
    let script = app.join("Scripts/preinstall");
    fs::remove_file(&script).map_err(|e| format!("{e} removing {}", script.display()))?;
    let dc = expanded
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("AcroRdrDC"));
    fs::write(
        &script,
        if dc {
            include_bytes!("community_readerdc_preinstall").as_slice()
        } else {
            include_bytes!("community_reader_preinstall").as_slice()
        },
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    let resource = if dc {
        "readerdc_preinstall"
    } else {
        "reader_preinstall"
    };
    output(1, format!("Replaced pkg preinstall script with our custom script at embedded:AdobeReader/package_resources/scripts/{resource}"));
    Ok(())
}
fn pkgutil(args: &[&std::ffi::OsStr]) -> Result<()> {
    let result = Command::new("/usr/sbin/pkgutil")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "pkgutil failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ))
    }
}
fn repackager(env: &mut Dictionary) -> Result<()> {
    let mut mount = super::dmg::Mount::new(super::string(env, "dmg_path")?)?;
    let result = (|| {
        let pkg = fs::read_dir(mount.path())
            .map_err(|e| e.to_string())?
            .filter_map(|p| p.ok())
            .map(|p| p.path())
            .find(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(".pkg"))
            })
            .ok_or_else(|| format!("No package found in {}", mount.path().display()))?;
        let cache = Path::new(super::string(env, "RECIPE_CACHE_DIR")?);
        let expanded = cache.join(pkg.file_stem().ok_or("Invalid package name")?);
        let output = cache.join(pkg.file_name().ok_or("Invalid package name")?);
        if expanded.is_dir() {
            fs::remove_dir_all(&expanded).map_err(|e| e.to_string())?;
        }
        pkgutil(&["--expand".as_ref(), pkg.as_os_str(), expanded.as_os_str()])?;
        modify_distribution(&expanded.join("Distribution"))?;
        replace_preinstall(&expanded)?;
        if output.exists() {
            fs::remove_file(&output).map_err(|e| e.to_string())?;
        }
        pkgutil(&[
            "--flatten".as_ref(),
            expanded.as_os_str(),
            output.as_os_str(),
        ])?;
        env.insert(
            "pkg_path".into(),
            output.to_string_lossy().into_owned().into(),
        );
        Ok(())
    })();
    mount.detach().and(result)
}
fn sassafras(env: &Dictionary) -> Result<()> {
    let script = super::string(env, "k2clientconfig_path")?;
    let pkg = super::string(env, "base_pkg_path")?;
    if !Path::new(script).exists() {
        return Err(format!("No file exists at k2clientconfig_path: {script}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(script)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            fs::set_permissions(script, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }
    if !Path::new(pkg).exists() {
        return Err(format!("No K2Client pkg exists at base_pkg_path: {pkg}"));
    }
    let result = Command::new(script)
        .args(super::string(env, "k2clientconfig_options")?.split_whitespace())
        .arg(pkg)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.stderr.is_empty() {
        return Err(format!(
            "k2clientconfig returned errors:\n{}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(())
}
pub(super) fn execute(
    name: &str,
    env: &mut Dictionary,
    _preferences: Option<&Dictionary>,
) -> Result<()> {
    match name {
        "AdobeReaderURLProvider" => reader(env).map_err(|e| e.message),
        "AdobeFlashURLProvider" => flash(env),
        "AdobeAcrobatProUpdateInfoProvider" => acrobat(env, fetch).map_err(|e| e.message),
        "PuppetlabsProductsURLProvider" => puppet(env),
        "AdobeReaderRepackager" => repackager(env),
        "SassafrasK2ClientCustomizer" => sassafras(env),
        _ => Err(format!("Unknown legacy processor: {name}")),
    }
}

/// Preserve the Python runtime-exception boundary at the failing operation.
pub(super) fn execute_typed(name: &str, env: &mut Dictionary) -> TypedResult<()> {
    match name {
        "AdobeReaderURLProvider" => reader(env),
        "AdobeAcrobatProUpdateInfoProvider" => acrobat(env, fetch),
        _ => Err(format!("Unknown typed legacy processor: {name}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_errors_are_typed_without_reclassifying_processor_failures() {
        let (url, server) = crate::downloader::tests::server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),
        ]);
        let mut reader_env = Dictionary::from_iter([("base_url", url.as_str())]);
        let error =
            crate::execute_standalone("AdobeReaderURLProvider", &mut reader_env).unwrap_err();
        assert_eq!(error.kind, crate::FailureKind::Unexpected);
        assert!(error.message.contains("Missing reader product"));
        server.join().unwrap();
        let mut env = Dictionary::from_iter([("major_version", "11")]);
        let error = acrobat(&mut env, |_, url| {
            if url.ends_with(".txt") {
                return Ok("/current.plist".into());
            }
            let manifest = Dictionary::from_iter([
                ("PatchURL", "/patch.dmg"),
                ("BuildNumber", "11.0.3"),
                ("PreviousURLTemplate", "noTemplate"),
            ]);
            let mut bytes = Vec::new();
            Value::Dictionary(manifest)
                .to_writer_xml(&mut bytes)
                .unwrap();
            Ok(String::from_utf8(bytes).unwrap())
        })
        .unwrap_err();
        assert_eq!(error.kind, crate::FailureKind::Unexpected);
        assert!(error.message.contains("prev_version"));
        env.insert("major_version".into(), "12".into());
        let error =
            crate::execute_standalone("AdobeAcrobatProUpdateInfoProvider", &mut env).unwrap_err();
        assert_eq!(error.kind, crate::FailureKind::Processor);
        env.insert("major_version".into(), "11".into());
        let error = acrobat(&mut env, |_, _| Err("curl failed".into())).unwrap_err();
        assert_eq!(error.kind, crate::FailureKind::Processor);
    }
    #[test]
    fn flash_xml_explicit_version_and_override() {
        assert_eq!(
            flash_version(r#"<XML><update version="32,0,0,465"/></XML>"#).unwrap(),
            "32,0,0,465"
        );
        assert!(flash_version("<other><update version='1'/></other>").is_err());
        let mut e = Dictionary::from_iter([("version", "32,0,0,465")]);
        flash(&mut e).unwrap();
        assert_eq!(e["url"].as_string(),Some("https://fpdownload.macromedia.com/get/flashplayer/pdc/32.0.0.465/install_flash_player_osx.dmg"));
        e.insert("url".into(), "override".into());
        flash(&mut e).unwrap();
        assert_eq!(e["url"].as_string(), Some("override"));
    }
    #[test]
    fn puppet_latest_specific_and_agent() {
        let mut e = Dictionary::from_iter([("product_name", "Puppet")]);
        let index = r#"<a href="puppet-3.9.dmg">old</a><a href="puppet-3.10.dmg">new</a><a href="puppet-4.0-rc.dmg">preview</a>"#;
        assert_eq!(puppet_candidate(&e, index).unwrap().1, "3.10");
        e.insert("get_version".into(), "3.9".into());
        assert_eq!(puppet_candidate(&e, index).unwrap().1, "3.9");
        e.insert("product_name".into(), "agent".into());
        assert_eq!(
            puppet_candidate(&e, r#"href="puppet-agent-1.2.5-1.osx10.10.dmg""#)
                .unwrap()
                .1,
            "1.2.5"
        );
        assert!(puppet_candidate(&e, "no candidates").is_err());
    }
    #[test]
    fn acrobat_templates_manifest_and_required_update() {
        let mut e = Dictionary::from_iter([("major_version", "11"), ("version", "11.0.3")]);
        let mut urls = Vec::new();
        acrobat(&mut e, |_, url| {
            urls.push(url.to_owned());
            if url.ends_with(".txt") {
                return Ok("/{MAJREV}/11.0.99/{PROD}_{PROD_ARCH}_{OS_VER_MIN}.plist".into());
            }
            let mut data = Dictionary::from_iter([
                ("PatchURL", "/update.dmg"),
                ("BuildNumber", "11.0.3"),
                ("PreviousURLTemplate", "/previous.plist"),
            ]);
            if url.ends_with("previous.plist") {
                data.insert("BuildNumber".into(), "11.0.2".into());
            }
            let mut out = Vec::new();
            Value::Dictionary(data).to_writer_xml(&mut out).unwrap();
            Ok(String::from_utf8(out).unwrap())
        })
        .unwrap();
        assert!(urls[1].ends_with("/11/11.0.3/com_adobe_Acrobat_Pro_univ_9.plist"));
        assert_eq!(
            e["url"].as_string(),
            Some("http://armdl.adobe.com/update.dmg")
        );
        let info = e["additional_pkginfo"].as_dictionary().unwrap();
        assert_eq!(
            info["requires"].as_array().unwrap()[0].as_string(),
            Some("AdobeAcrobatPro11_Update-11.0.2")
        );
        assert_eq!(info["minimum_os_version"].as_string(), Some("10.9.0"));
        e.insert("major_version".into(), "12".into());
        assert!(acrobat(&mut e, |_, _| panic!("must reject before network")).is_err());
    }
    #[test]
    fn reader_local_http_quoted_parameters_and_outputs() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                loop {
                    let n = socket.read(&mut buf).unwrap();
                    request.extend_from_slice(&buf[..n]);
                    if n == 0 || request.windows(4).any(|p| p == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                assert!(request.contains("Mac%20OS%2010.14.0"));
                let body = if index == 0 {
                    r#"{"products":{"reader":[{"displayName":"Reader DC 2022 for Mac","version":"22.001.20112"}]}}"#
                } else {
                    assert!(request.contains("Reader%20DC%202022%20for%20Mac"));
                    r#"{"downloadURL":"https://example.invalid/reader.dmg","saveName":"reader.dmg"}"#
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let mut e = Dictionary::new();
        e.insert(
            "base_url".into(),
            format!("http://{address}/products?os={{OS_VERSION}}").into(),
        );
        e.insert(
            "download_url".into(),
            format!("http://{address}/download?os={{OS_VERSION}}&name={{DISPLAY_NAME}}").into(),
        );
        e.insert("os_version".into(), "10.14.0".into());
        e.insert(
            "curl_opts".into(),
            Value::Array(vec!["--not-a-valid-option".into()]),
        );
        e.insert("request_headers".into(), "unused, not a dictionary".into());
        reader(&mut e).unwrap();
        server.join().unwrap();
        assert_eq!(e["version"].as_string(), Some("22.001.20112"));
        assert_eq!(e["filename"].as_string(), Some("reader.dmg"));
        assert_eq!(
            e["url"].as_string(),
            Some("https://example.invalid/reader.dmg")
        );
    }
    #[test]
    fn repackager_modifies_distribution_and_replaces_correct_script() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["AcroRdrDC_22001", "AdobeReaderXI"] {
            let expanded = temp.path().join(name);
            fs::create_dir_all(expanded.join("application_mini_7z.pkg/Scripts")).unwrap();
            let dist = expanded.join("Distribution");
            fs::write(&dist,r#"<installer-gui-script><domains enable_anywhere="false"/><options customize="never"/></installer-gui-script>"#).unwrap();
            modify_distribution(&dist).unwrap();
            let text = fs::read_to_string(&dist).unwrap();
            assert!(!text.contains("domains"));
            assert!(text.contains("customize=\"never\""));
            let script = expanded.join("application_mini_7z.pkg/Scripts/preinstall");
            fs::write(&script, "bad script").unwrap();
            replace_preinstall(&expanded).unwrap();
            let text = fs::read_to_string(&script).unwrap();
            assert!(text.contains(if name.starts_with("AcroRdrDC") {
                "Adobe Acrobat Reader DC.app"
            } else {
                "Adobe Reader.app"
            }));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(&script).unwrap().permissions().mode() & 0o777,
                    0o755
                );
            }
        }
        let path = temp.path().join("wrong");
        fs::write(&path, "<wrong/>").unwrap();
        assert_eq!(
            modify_distribution(&path).unwrap_err(),
            "Distribution file is not in the expected format."
        );
    }
    #[cfg(unix)]
    #[test]
    fn sassafras_permissions_arguments_and_stderr_semantics() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("config");
        let pkg = temp.path().join("Client Package.pkg");
        fs::write(&pkg, "").unwrap();
        fs::write(&script,"#!/bin/sh\n[ \"$1\" = '-s' ] && [ \"$2\" = 'server' ] || exit 8\nprintf '%s' \"$3\" > \"$3\"\nexit 4\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
        let mut e = Dictionary::new();
        e.insert(
            "base_pkg_path".into(),
            pkg.to_string_lossy().into_owned().into(),
        );
        e.insert(
            "k2clientconfig_path".into(),
            script.to_string_lossy().into_owned().into(),
        );
        e.insert("k2clientconfig_options".into(), "-s  server".into());
        sassafras(&e).unwrap();
        assert_eq!(fs::read_to_string(&pkg).unwrap(), pkg.to_string_lossy());
        assert_eq!(
            fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::write(&script, "#!/bin/sh\necho warning >&2\n").unwrap();
        assert!(sassafras(&e)
            .unwrap_err()
            .contains("k2clientconfig returned errors"));
    }
}
