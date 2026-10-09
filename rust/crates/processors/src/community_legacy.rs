//! Native ports of the pinned autopkg/recipes Adobe, Puppet and Sassafras processors.
//! Original processors: Timothy Sutton, Per Olofsson, Glynn Lane, Nate Felton,
//! Greg Neagle and Allister Banks, Apache-2.0.
use plist::{Dictionary, Value};
pub(crate) type Result<T> = std::result::Result<T, String>;
pub(crate) type TypedResult<T> = std::result::Result<T, super::ExecutionFailure>;
pub(crate) fn output(level: i64, message: impl std::fmt::Display) {
    autopkg_platform::processor_output(level, message);
}
pub(crate) fn get<'a>(env: &'a Dictionary, key: &str, default: &'a str) -> &'a str {
    env.get(key).and_then(Value::as_string).unwrap_or(default)
}
pub(crate) fn fetch(env: &Dictionary, url: &str) -> Result<String> {
    let mut request = env.clone();
    // URLGetter.download passes only its explicit headers. Flash alone calls
    // add_curl_common_opts and therefore inherits recipe headers/options.
    request.remove("request_headers");
    request.remove("curl_opts");
    fetch_common(&request, url)
}
pub(crate) fn fetch_common(env: &Dictionary, url: &str) -> Result<String> {
    let mut request = env.clone();
    request.insert("url".into(), url.into());
    crate::processors::url_getter::fetch(&request)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::processors::{
        adobe_acrobat_pro_update_info_provider::acrobat,
        adobe_flash_url_provider::{execute as flash, flash_version},
        adobe_reader_repackager::{modify_distribution, replace_preinstall},
        adobe_reader_url_provider::execute_typed as reader,
        puppetlabs_products_url_provider::puppet_candidate,
    };
    #[cfg(unix)]
    use crate::processors::sassafras_k2_client_customizer::execute as sassafras;
    use std::fs;
    #[test]
    fn runtime_errors_are_typed_without_reclassifying_processor_failures() {
        let (url, server) = crate::processors::url_downloader::tests::server(vec![
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
