use super::download_transport::{self, Headers};
use super::{io, json_value, string, Result};
use autopkg_platform::processor_output as output;
use plist::{Dictionary, Value};
use serde_json::{json, Value as Json};
use sha2::Digest;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
#[path = "download_trust.rs"]
pub(super) mod trust;
fn repr(value: &Json) -> String {
    plist::python_repr(&json_value(value).unwrap_or(Value::Null))
}
fn python_metadata(value: &Json, env: &Dictionary) -> Value {
    let mut metadata = Dictionary::new();
    for key in [
        "file_name",
        "file_size",
        "file_sha1",
        "file_sha256",
        "file_md5",
        "download_url",
    ] {
        if let Some(value) = value.get(key) {
            metadata.insert(key.into(), json_value(value).unwrap_or(Value::Null));
        }
    }
    let mut headers = Dictionary::new();
    let mut names = vec![
        "ETag".to_string(),
        "Last-Modified".into(),
        "Content-Length".into(),
    ];
    if let Some(Value::Array(extra)) = env.get("HEADERS_TO_TEST") {
        names.extend(
            extra
                .iter()
                .filter_map(Value::as_string)
                .map(str::to_string),
        );
    }
    for key in names {
        let canonical = match key.to_lowercase().as_str() {
            "etag" => "ETag",
            "last-modified" => "Last-Modified",
            "content-length" => "Content-Length",
            _ => &key,
        };
        if let Some(value) = value["http_headers"].get(canonical) {
            headers.insert(canonical.into(), json_value(value).unwrap_or(Value::Null));
        }
    }
    metadata.insert("http_headers".into(), headers.into());
    metadata.into()
}
fn boolean(env: &Dictionary, key: &str, default: bool) -> Result<bool> {
    match env.get(key) {
        None => Ok(default),
        Some(Value::Boolean(b)) => Ok(*b),
        Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Ok(true),
            "false" | "no" | "off" | "0" | "" => Ok(false),
            _ => Err(format!("{key} must be a boolean or boolean-like string")),
        },
        _ => Err(format!("{key} must be a boolean or boolean-like string")),
    }
}
fn python_text(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Boolean(b) => Ok(if *b { "True" } else { "False" }.into()),
        Value::Integer(n) => Ok(n.to_string()),
        Value::Real(n) => Ok(n.to_string()),
        _ => Err("Header values must be scalar values".into()),
    }
}
fn command(
    env: &Dictionary,
    python: bool,
    operation: &[std::ffi::OsString],
) -> std::result::Result<(Command, Option<trust::Bundle>), super::ExecutionFailure> {
    let mut c = Command::new(autopkg_platform::downloads::curl_binary(env)?);
    c.args([
        "--silent",
        "--show-error",
        "--no-buffer",
        "--dump-header",
        "-",
        "--speed-time",
        "30",
        "--location",
        "--url",
        string(env, "url")?,
    ]);
    c.args(operation);
    if let Some(headers) = env.get("request_headers") {
        for (key, value) in headers
            .as_dictionary()
            .ok_or("request_headers must be a dictionary")?
        {
            c.arg("--header")
                .arg(format!("{key}: {}", python_text(value)?));
        }
    }
    if !python {
        if let Some(options) = env.get("curl_opts") {
            for option in options.as_array().ok_or("curl_opts must be an array")? {
                c.arg(
                    option
                        .as_string()
                        .ok_or("curl_opts entries must be strings")?,
                );
            }
        }
    }
    let bundle = if python {
        if let Some(cafile) = trust::explicit_certificate() {
            output(1, format!("SSL_CERT_FILE={}", cafile.display()));
        }
        let bundle = trust::bundle()?;
        if let Some(cafile) = &bundle.implicit_certificate {
            output(1, format!("SSL_CERT_FILE={}", cafile.display()));
        }
        c.arg("--cacert").arg(bundle.path());
        if let Some(path) = &bundle.capath {
            c.arg("--capath").arg(path);
        }
        Some(bundle)
    } else {
        trust::native_curl(&mut c)?
    };
    Ok((c, bundle))
}
fn info_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.info.json", path.display()))
}
#[cfg(unix)]
fn xattr_names() -> [String; 2] {
    let prefix = if cfg!(target_os = "linux") {
        "user."
    } else {
        ""
    };
    [
        format!("{prefix}com.github.autopkg.etag"),
        format!("{prefix}com.github.autopkg.last-modified"),
    ]
}
fn metadata(path: &Path, python: bool, logging: bool) -> Json {
    match fs::read(info_path(path)) {
        Ok(bytes) => match serde_json::from_slice::<Json>(&bytes) {
            Ok(value) => {
                if logging {
                    output(2, "Reading metadata from Info JSON.");
                    output(2, format!("Info JSON contents: {}", repr(&value)));
                }
                if value.is_object() {
                    value
                } else {
                    json!({})
                }
            }
            Err(error) => {
                if logging {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut column = error.column().max(1);
                    let mut position = text
                        .lines()
                        .take(error.line().saturating_sub(1))
                        .map(|s| s.chars().count() + 1)
                        .sum::<usize>()
                        + column
                        - 1;
                    let message = error.to_string();
                    if message.starts_with("expected ident") {
                        let chars = text.chars().collect::<Vec<_>>();
                        while position > 0
                            && chars
                                .get(position - 1)
                                .is_some_and(char::is_ascii_alphabetic)
                        {
                            position -= 1;
                            column = column.saturating_sub(1).max(1);
                        }
                    }
                    let reason = if message.starts_with("key must be a string") {
                        "Expecting property name enclosed in double quotes"
                    } else if message.starts_with("trailing characters") {
                        "Extra data"
                    } else if message.starts_with("expected `:`") {
                        "Expecting ':' delimiter"
                    } else if message.starts_with("expected `,`") {
                        "Expecting ',' delimiter"
                    } else {
                        "Expecting value"
                    };
                    output(1, format!("WARNING: Could not read {} (JSONDecodeError): {reason}: line {} column {column} (char {position}). Continuing with empty metadata.", info_path(path).display(), error.line()));
                }
                json!({})
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            if !python && path.is_file() {
                let names = xattr_names();
                let mut headers = serde_json::Map::new();
                for (key, name) in ["ETag", "Last-Modified"].iter().zip(names.iter()) {
                    if let Ok(Some(value)) = xattr::get(path, name) {
                        if let Ok(value) = String::from_utf8(value) {
                            headers.insert((*key).into(), value.into());
                        }
                    }
                }
                if !headers.is_empty() {
                    if logging {
                        output(2, "Reading metadata from pre-3.0 xattrs.");
                    }
                    return json!({"http_headers":headers});
                }
            }
            let _ = python;
            json!({})
        }
        Err(error) => {
            if logging {
                let (kind, message) = if error.kind() == std::io::ErrorKind::PermissionDenied {
                    ("PermissionError", "Permission denied")
                } else if error.raw_os_error() == Some(21) {
                    ("IsADirectoryError", "Is a directory")
                } else {
                    ("OSError", "Input/output error")
                };
                output(1, format!("WARNING: Could not read {} ({kind}): [Errno {}] {message}: {}. Continuing with empty metadata.", info_path(path).display(), error.raw_os_error().unwrap_or(5), plist::python_quote(&info_path(path).to_string_lossy())));
            }
            json!({})
        }
    }
}
fn publish(env: &mut Dictionary, path: &Path, metadata: &Json) -> Result<()> {
    let size = fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .or_else(|| metadata["file_size"].as_u64())
        .unwrap_or(0);
    env.insert("file_size".into(), size.into());
    if metadata.as_object().is_some_and(|m| !m.is_empty()) {
        env.insert("download_info".into(), json_value(metadata)?);
        env.insert(
            "etag".into(),
            metadata["http_headers"]["ETag"]
                .as_str()
                .unwrap_or("")
                .into(),
        );
        env.insert(
            "last_modified".into(),
            metadata["http_headers"]["Last-Modified"]
                .as_str()
                .unwrap_or("")
                .into(),
        );
        env.insert(
            "download_url".into(),
            metadata["download_url"].as_str().unwrap_or("").into(),
        );
    }
    Ok(())
}
fn test_headers(env: &Dictionary) -> Result<Vec<String>> {
    if boolean(env, "CHECK_FILESIZE_ONLY", false)? {
        return Ok(vec!["Content-Length".into()]);
    }
    let defaults = vec![
        "ETag".into(),
        "Last-Modified".into(),
        "Content-Length".into(),
    ];
    match env.get("HEADERS_TO_TEST") {
        Some(Value::Array(a)) if !a.is_empty() => a
            .iter()
            .map(|v| {
                v.as_string()
                    .map(str::to_string)
                    .ok_or("HEADERS_TO_TEST entries must be strings".into())
            })
            .collect(),
        None | Some(Value::Array(_)) => Ok(defaults),
        _ => Err("HEADERS_TO_TEST must be an array".into()),
    }
}
fn json_header<'a>(metadata: &'a Json, key: &str) -> Option<&'a Json> {
    metadata["http_headers"]
        .as_object()?
        .iter()
        .find_map(|(k, v)| k.eq_ignore_ascii_case(key).then_some(v))
}
fn integer(value: &Json) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn changed(env: &Dictionary, path: &Path, metadata: &Json, headers: &Headers) -> Result<bool> {
    if headers.get("http_result_code").is_some_and(|s| s == "304") {
        output(1, "Item at URL is unchanged.");
        return Ok(false);
    }
    let existing = fs::metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len());
    let mut matches = 0;
    for key in test_headers(env)? {
        let previous = json_header(metadata, &key);
        let current = headers.get(&key.to_lowercase());
        if key.eq_ignore_ascii_case("Content-Length") {
            let previous = existing.or_else(|| previous.and_then(integer));
            let current = current.and_then(|s| s.parse::<u64>().ok());
            if let (Some(a), Some(b)) = (previous, current) {
                if a != b {
                    output(2, "Content-Length is different");
                    return Ok(true);
                }
                matches += 1;
            } else {
                output(1, "WARNING: 'Content-Length' missing. (TypeError) int() argument must be a string, a bytes-like object or a real number, not 'NoneType'");
            }
            continue;
        }
        if current.is_none() && previous.is_none_or(|p| p.is_null() || p.as_str() == Some("")) {
            continue;
        }
        let Some(previous) = previous else {
            output(1, format!("WARNING: header missing. (KeyError) {key}"));
            continue;
        };
        if previous.as_str() != current.map(String::as_str) {
            output(2, format!("{key} is different"));
            return Ok(true);
        }
        matches += 1;
    }
    if matches > 0 {
        return Ok(false);
    }
    if !headers.contains_key("etag") && !headers.contains_key("last-modified") {
        if let (Some(a), Some(b)) = (
            existing,
            headers
                .get("content-length")
                .and_then(|s| s.parse::<u64>().ok()),
        ) {
            if a == b {
                output(1, format!("File size returned by webserver matches that of the cached file: {b} bytes"));
                output(1, "WARNING: Matching a download by filesize is a fallback mechanism that does not guarantee that a build is unchanged.");
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn hashes(env: &mut Dictionary, path: &Path) -> Result<()> {
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut md5 = md5::Md5::new();
    let mut file = io(fs::File::open(path))?;
    let mut bytes = [0; 4096 * 100];
    loop {
        let count = io(file.read(&mut bytes))?;
        if count == 0 {
            break;
        }
        sha1.update(&bytes[..count]);
        sha256.update(&bytes[..count]);
        md5.update(&bytes[..count]);
    }
    env.insert("file_sha1".into(), format!("{:x}", sha1.finalize()).into());
    env.insert(
        "file_sha256".into(),
        format!("{:x}", sha256.finalize()).into(),
    );
    env.insert("file_md5".into(), format!("{:x}", md5.finalize()).into());
    Ok(())
}
fn existing_hashes(env: &mut Dictionary, path: &Path, metadata: &Json) -> Result<()> {
    if !boolean(env, "COMPUTE_HASHES", false)? {
        return Ok(());
    }
    if path.is_file() {
        return hashes(env, path);
    }
    let keys = ["file_sha1", "file_sha256", "file_md5"];
    if keys
        .iter()
        .all(|key| metadata[*key].as_str().is_some_and(|s| !s.is_empty()))
    {
        for key in keys {
            env.insert(key.into(), metadata[key].as_str().unwrap().into());
        }
        output(2, "Reusing hashes from .info.json (cached file absent).");
    } else {
        output(1, "WARNING: COMPUTE_HASHES is set but the cached file is absent and no stored hashes were found in .info.json; skipping hashes.");
    }
    Ok(())
}
fn download_dir(env: &Dictionary) -> Result<PathBuf> {
    let dir = match env
        .get("download_dir")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
    {
        Some(path) => PathBuf::from(path),
        None => Path::new(string(env, "RECIPE_CACHE_DIR")?).join("downloads"),
    };
    io(fs::create_dir_all(&dir))?;
    Ok(dir)
}
fn stage(env: &mut Dictionary, source: &str) -> Result<()> {
    let source = if let Some(rest) = source.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(rest)
    } else {
        PathBuf::from(source)
    };
    if !source.exists() {
        return Err(format!("PKG path {} does not exist", source.display()));
    }
    let destination =
        download_dir(env)?.join(source.file_name().ok_or("PKG path has no basename")?);
    if source.canonicalize().ok() != destination.canonicalize().ok() {
        if fs::symlink_metadata(&destination).is_ok() {
            super::remove(&destination)?;
        }
        if info_path(&destination).exists() {
            io(fs::remove_file(info_path(&destination)))?;
        }
        if cfg!(target_os = "macos")
            && Command::new("/bin/cp")
                .arg("-Rc")
                .arg(&source)
                .arg(&destination)
                .output()
                .is_ok_and(|o| o.status.success())
        {
        } else if source.is_dir() {
            super::copy_tree(&source, &destination)?;
        } else {
            io(fs::copy(&source, &destination))?;
            // Match shutil.copy2 when clonefile is unavailable: retain unrelated
            // extended attributes and timestamps before clearing legacy headers.
            super::copy_metadata(&source, &destination, false)?;
        }
        #[cfg(unix)]
        for name in xattr_names() {
            let _ = xattr::remove(&destination, name);
        }
    }
    env.insert(
        "pathname".into(),
        destination.to_string_lossy().into_owned().into(),
    );
    env.insert("download_changed".into(), true.into());
    output(
        1,
        format!("Given {}, no download needed.", source.display()),
    );
    if source != destination {
        output(
            1,
            format!("Staged {} to {}", source.display(), destination.display()),
        );
    }
    Ok(())
}
fn store(
    env: &mut Dictionary,
    path: &Path,
    headers: &Headers,
    url: &str,
    python: bool,
) -> Result<()> {
    let size = io(fs::metadata(path))?.len();
    let mut persisted = serde_json::Map::new();
    let mut names = vec![
        "ETag".to_string(),
        "Last-Modified".into(),
        "Content-Length".into(),
    ];
    if let Some(Value::Array(a)) = env.get("HEADERS_TO_TEST") {
        for v in a {
            names.push(
                v.as_string()
                    .ok_or("HEADERS_TO_TEST entries must be strings")?
                    .into(),
            );
        }
    }
    for name in names {
        let canonical = match name.to_lowercase().as_str() {
            "etag" => "ETag",
            "last-modified" => "Last-Modified",
            "content-length" => "Content-Length",
            _ => &name,
        };
        let value = if name.eq_ignore_ascii_case("Content-Length") {
            json!(size)
        } else {
            json!(headers
                .get(&name.to_lowercase())
                .map(String::as_str)
                .unwrap_or(""))
        };
        persisted.insert(canonical.into(), value);
    }
    let mut metadata = json!({"download_url":url,"file_name":path.file_name().unwrap_or_default().to_string_lossy(),"file_size":size,"http_headers":persisted});
    if boolean(env, "COMPUTE_HASHES", false)? {
        hashes(env, path)?;
        for key in ["file_sha1", "file_sha256", "file_md5"] {
            metadata[key] = json!(env[key].as_string().unwrap());
        }
    }
    publish(env, path, &metadata)?;
    if python {
        let ordered = python_metadata(&metadata, env);
        env.insert("download_info".into(), ordered);
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Download path has no parent")?)
            .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    serde::Serialize::serialize(&metadata, &mut serializer).map_err(|e| e.to_string())?;
    if python {
        if headers
            .get("content-length")
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n != size)
        {
            output(1, "WARNING: file size != content-length header");
        }
        save_headers(path, headers, true)?;
        output(
            2,
            format!(
                "download_dictionary: \n{}\n",
                plist::python_repr(&python_metadata(&metadata, env))
            ),
        );
    }
    output(
        1,
        format!("Storing metadata to {}", info_path(path).display()),
    );
    output(
        2,
        format!("Metadata contents:\n{}", String::from_utf8_lossy(&bytes)),
    );
    if cfg!(windows) {
        // Python writes the JSON sidecar in text mode; Windows translates only
        // physical line endings, not the escaped newlines inside JSON strings.
        bytes = String::from_utf8(bytes)
            .map_err(|e| e.to_string())?
            .replace('\n', "\r\n")
            .into_bytes();
    }
    io(temporary.write_all(&bytes))?;
    temporary
        .persist(info_path(path))
        .map_err(|e| e.to_string())?;
    if !python {
        save_headers(path, headers, false)?;
    }
    Ok(())
}
fn save_headers(path: &Path, headers: &Headers, python: bool) -> Result<()> {
    for (key, label, index) in [("last-modified", "Last-Modified", 1), ("etag", "ETag", 0)] {
        if let Some(value) = headers.get(key).filter(|v| !v.is_empty()) {
            #[cfg(unix)]
            if let Err(error) = xattr::set(path, &xattr_names()[index], value.as_bytes()) {
                if !python {
                    return Err(error.to_string());
                }
                let kind = match error.kind() {
                    std::io::ErrorKind::PermissionDenied => "PermissionError",
                    std::io::ErrorKind::NotFound => "FileNotFoundError",
                    _ => "OSError",
                };
                let text = error.to_string();
                let reason = text.split(" (os error").next().unwrap_or(&text);
                output(
                    1,
                    format!(
                        "ERROR xattr: ({kind})\n[Errno {}] {reason}: {}\n",
                        error.raw_os_error().unwrap_or(5),
                        plist::python_quote(&path.to_string_lossy())
                    ),
                );
                return Ok(());
            }
            let _ = (path, python, index);
            output(1, format!("Storing new {label} header: {value}"));
        }
    }
    Ok(())
}
pub(super) fn execute(name: &str, env: &mut Dictionary) -> Result<()> {
    execute_typed(name, env).map_err(|e| e.message)
}
pub(super) fn execute_typed(
    name: &str,
    env: &mut Dictionary,
) -> std::result::Result<(), super::ExecutionFailure> {
    let python = name == "URLDownloaderPython";
    env.remove("url_downloader_summary_result");
    env.insert("file_size".into(), 0.into());
    for key in ["last_modified", "etag", "download_url"] {
        env.insert(key.into(), "".into());
    }
    env.insert("download_info".into(), Dictionary::new().into());
    if env.contains_key("PKG") {
        let source = string(env, "PKG")?.to_string();
        return stage(env, &source).map_err(Into::into);
    }
    let url = string(env, "url")?.to_string();
    let mut filename = None;
    if boolean(env, "prefetch_filename", false)? {
        let (command, _trust) = command(env, false, &["--head".into()])?;
        let (headers, _) = download_transport::run(command, false)?;
        if let Some(disposition) = headers
            .get("content-disposition")
            .filter(|s| s.contains("filename="))
        {
            filename = Some(
                disposition
                    .rsplit_once("filename=")
                    .unwrap()
                    .1
                    .replace('"', "")
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            );
            output(
                2,
                format!(
                    "Filename prefetched from the HTTP Content-Disposition header: {}",
                    filename.as_deref().unwrap_or("")
                ),
            );
        } else if let Some(redirect) = headers.get("http_redirected") {
            filename = Some(redirect.rsplit('/').next().unwrap_or("").to_string());
            output(
                2,
                format!(
                    "Filename prefetched from the HTTP Location header: {}",
                    filename.as_deref().unwrap_or("")
                ),
            );
        } else {
            output(
                2,
                "Unable to find filename in the HTTP headers during prefetch",
            );
        }
    }
    let filename = filename.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        env.get("filename")
            .and_then(Value::as_string)
            .unwrap_or_else(|| url.rsplit('/').next().unwrap_or(""))
            .to_string()
    });
    if python {
        env.insert("filename".into(), filename.clone().into());
    }
    let dir = download_dir(env)?;
    let path = dir.join(filename);
    env.insert(
        "pathname".into(),
        path.to_string_lossy().into_owned().into(),
    );
    if path.is_file() && io(fs::metadata(&path))?.len() == 0 {
        io(fs::remove_file(&path))?;
    }
    let previous = metadata(&path, python, !python && path.exists());
    let mut temporary = tempfile::Builder::new()
        .prefix("tmp")
        .tempfile_in(&dir)
        .map_err(|e| e.to_string())?;
    super::mode(temporary.path(), "644")?;
    let (mut command, _trust) = match command(
        env,
        python,
        &[
            "--fail".into(),
            "--output".into(),
            temporary.path().as_os_str().to_owned(),
        ],
    ) {
        Ok(command) => command,
        Err(error) => {
            // Python creates its delete=False transfer file before validating
            // the certificate override, so setup failures retain it as well.
            let _ = temporary.keep().map_err(|e| e.to_string())?;
            return Err(error);
        }
    };
    if !python && path.exists() && !boolean(env, "CHECK_FILESIZE_ONLY", false)? {
        for (key, request) in [
            ("ETag", "If-None-Match"),
            ("Last-Modified", "If-Modified-Since"),
        ] {
            if let Some(value) = previous["http_headers"][key]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                command.arg("--header").arg(format!("{request}: {value}"));
            }
        }
    }
    if python {
        command.args(["--write-out", "\nAUTOPKG_EFFECTIVE_URL:%{url_effective}"]);
    }
    let (headers, effective) = match download_transport::run(command, python) {
        Ok(result) => result,
        Err(error) => {
            if let Some(headers) = error.incomplete.as_ref().filter(|_| python) {
                // urllib publishes response metadata before streaming the body,
                // including when the subsequent chunked read fails.
                let previous = metadata(&path, true, true);
                publish(env, &path, &previous)?;
                let version_changed = changed(env, &path, &previous, headers)?;
                env.insert("download_changed".into(), version_changed.into());
                // urllib writes only completed read(409600) chunks before an
                // IncompleteRead exception; curl also writes the final fragment.
                let length = io(temporary.as_file().metadata())?.len();
                io(temporary.as_file().set_len(length / 409600 * 409600))?;
            }
            // The pinned downloaders retain their delete=False temporary file
            // on transfer failure; preserve that observable cache state.
            let _ = temporary.keep().map_err(|e| e.to_string())?;
            return Err(error.failure);
        }
    };
    let previous = metadata(&path, python, true);
    publish(env, &path, &previous)?;
    let changed = changed(env, &path, &previous, &headers)?;
    env.insert("download_changed".into(), changed.into());
    let materialize = !path.is_file() && boolean(env, "DOWNLOAD_MISSING_FILE", true)?;
    if !changed && !materialize {
        existing_hashes(env, &path, &previous)?;
        if python {
            output(2, "download_dictionary: \nNone\n");
            output(
                4,
                format!(
                    "self.env: \n{}\n",
                    plist::python_repr(&Value::Dictionary(env.clone()))
                ),
            );
        } else if path.is_file() {
            output(1, format!("Using existing {}", path.display()));
        }
        return Ok(());
    }
    io(temporary.flush())?;
    if path.exists() {
        io(fs::remove_file(&path))?;
    }
    temporary.persist(&path).map_err(|e| e.to_string())?;
    let downloaded_url = if python && !effective.is_empty() {
        effective.as_str()
    } else {
        headers
            .get("http_redirected")
            .map(String::as_str)
            .unwrap_or(&url)
    };
    store(env, &path, &headers, downloaded_url, python)?;
    output(
        1,
        if changed {
            format!("Downloaded {}", path.display())
        } else {
            format!("Re-downloaded missing file: {}", path.display())
        },
    );
    let mut data = Dictionary::new();
    data.insert(
        "download_path".into(),
        path.to_string_lossy().into_owned().into(),
    );
    let mut summary = Dictionary::new();
    summary.insert(
        "summary_text".into(),
        if changed {
            "The following new items were downloaded:"
        } else {
            "The following missing items were re-downloaded:"
        }
        .into(),
    );
    summary.insert("data".into(), data.into());
    env.insert("url_downloader_summary_result".into(), summary.into());
    if python {
        output(
            4,
            format!(
                "self.env: \n{}\n",
                plist::python_repr(&Value::Dictionary(env.clone()))
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::download_transport::curl_stderr;
    use super::*;
    use crate::tests::{env, Temp};
    use std::collections::BTreeMap;
    /// Phase 1 baseline: the exact arguments a native download runs. Any
    /// backend that replaces curl for a recipe must reproduce these, or the
    /// recipe stays on curl.
    #[test]
    fn generated_curl_arguments_are_stable() {
        use plist::Value;
        let curl = "/usr/bin/curl";
        if !std::path::Path::new(curl).is_file() {
            return;
        }
        let url = "https://example.com/app.pkg";
        let args = |e: &Dictionary, operation: &[&str]| -> Vec<String> {
            let operation: Vec<std::ffi::OsString> = operation.iter().map(Into::into).collect();
            let (command, _) = super::command(e, false, &operation).unwrap();
            command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect()
        };
        let base = [
            "--silent",
            "--show-error",
            "--no-buffer",
            "--dump-header",
            "-",
            "--speed-time",
            "30",
            "--location",
            "--url",
            url,
        ];
        let plain = env(&[("url", url), ("CURL_PATH", curl)]);
        assert_eq!(args(&plain, &[]), base);
        assert_eq!(args(&plain, &["--head"]), [&base[..], &["--head"]].concat());

        let mut full = plain.clone();
        let mut headers = Dictionary::new();
        headers.insert("User-Agent".into(), "Tester/1.0".into());
        full.insert("request_headers".into(), headers.into());
        full.insert(
            "curl_opts".into(),
            Value::Array(vec![
                Value::String("--insecure".into()),
                Value::String("--user-agent".into()),
                Value::String("Agent/2.0".into()),
            ]),
        );
        // Operation arguments come first, then request headers, then recipe
        // curl_opts, each in the order the recipe gives them.
        assert_eq!(
            args(&full, &["--head"]),
            [
                &base[..],
                &[
                    "--head",
                    "--header",
                    "User-Agent: Tester/1.0",
                    "--insecure",
                    "--user-agent",
                    "Agent/2.0",
                ],
            ]
            .concat()
        );
    }

    #[test]
    fn curl_errors_decode_text_newlines_without_losing_blank_lines() {
        assert_eq!(
            curl_stderr(b"curl: (22) failed\r\n\r\nsecond line\rthird line\n"),
            "curl: (22) failed\n\nsecond line\nthird line\n"
        );
    }
    #[cfg(unix)]
    #[test]
    fn local_package_retains_metadata_except_stale_download_headers() {
        for processor in ["URLDownloader", "URLDownloaderPython"] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source.pkg");
            fs::write(&source, b"local package").unwrap();
            let time = filetime::FileTime::from_unix_time(1_600_000_000, 0);
            filetime::set_file_times(&source, time, time).unwrap();
            let unrelated = if cfg!(target_os = "linux") {
                "user.org.autopkg.cache-fixture"
            } else {
                "org.autopkg.cache-fixture"
            };
            xattr::set(&source, unrelated, b"preserved\0\xff").unwrap();
            for name in xattr_names() {
                xattr::set(&source, name, b"stale").unwrap();
            }
            let download = root.path().join("downloads");
            fs::create_dir(&download).unwrap();
            let destination = download.join("source.pkg");
            fs::write(&destination, b"old package").unwrap();
            fs::write(info_path(&destination), b"stale metadata").unwrap();
            let mut env = Dictionary::from_iter([
                ("url", Value::String("http://127.0.0.1:1/unused".into())),
                ("PKG", Value::String(source.to_string_lossy().into_owned())),
                (
                    "download_dir",
                    Value::String(download.to_string_lossy().into_owned()),
                ),
            ]);
            crate::execute_standalone(processor, &mut env).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), b"local package");
            assert!(!info_path(&destination).exists());
            assert_eq!(
                filetime::FileTime::from_last_modification_time(
                    &fs::metadata(&destination).unwrap()
                ),
                time
            );
            assert_eq!(
                xattr::get(&destination, unrelated).unwrap().unwrap(),
                b"preserved\0\xff"
            );
            for name in xattr_names() {
                assert!(xattr::get(&destination, &name).unwrap().is_none());
                assert_eq!(xattr::get(&source, name).unwrap().unwrap(), b"stale");
            }
        }
    }
    #[test]
    fn python_transport_exceptions_are_typed() {
        for response in [
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n8\r\nabc",
        ] {
            let root = tempfile::tempdir().unwrap();
            let (url, thread) = server(vec![response.into()]);
            let mut env = Dictionary::from_iter([
                ("url", Value::String(url)),
                (
                    "download_dir",
                    root.path().to_string_lossy().into_owned().into(),
                ),
            ]);
            let error = crate::execute_standalone("URLDownloaderPython", &mut env).unwrap_err();
            assert_eq!(error.kind, crate::FailureKind::Unexpected);
            thread.join().unwrap();
        }
    }

    pub(crate) fn server(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/file.pkg", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "request timed out");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                // Accepted sockets inherit nonblocking mode on Windows.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut bytes = [0; 1024];
                loop {
                    let count = stream.read(&mut bytes).unwrap();
                    request.extend_from_slice(&bytes[..count]);
                    if count == 0 || request.windows(4).any(|b| b == b"\r\n\r\n") {
                        break;
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, thread)
    }
    fn ok() -> String {
        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nETag: \"v1\"\r\nLast-Modified: Thu, 01 Oct 2026 00:00:00 GMT\r\nConnection: close\r\n\r\ndata".into()
    }
    #[test]
    fn conditional_cache_missing_materialization_and_failed_transfer_state() {
        let t = Temp::new();
        let (url, server) = server(vec![
            ok(),
            "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\nConnection: close\r\n\r\n".into(),
            ok(),
            ok(),
            "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        ]);
        let mut e = env(&[("url", &url), ("download_dir", &t.path("downloads"))]);
        e.insert("COMPUTE_HASHES".into(), true.into());
        crate::execute("URLDownloader", &mut e).unwrap();
        assert_eq!(e["download_changed"].as_boolean(), Some(true));
        assert_eq!(
            e["file_md5"].as_string(),
            Some("8d777f385d3dfec8815d20f7496026dc")
        );
        let path = PathBuf::from(e["pathname"].as_string().unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"data");
        crate::execute("URLDownloader", &mut e).unwrap();
        assert_eq!(e["download_changed"].as_boolean(), Some(false));
        assert!(!e.contains_key("url_downloader_summary_result"));
        assert_eq!(e["etag"].as_string(), Some("\"v1\""));
        fs::remove_file(&path).unwrap();
        crate::execute("URLDownloader", &mut e).unwrap();
        assert_eq!(e["download_changed"].as_boolean(), Some(false));
        assert!(path.exists());
        assert_eq!(
            e["url_downloader_summary_result"].as_dictionary().unwrap()["summary_text"].as_string(),
            Some("The following missing items were re-downloaded:")
        );
        fs::remove_file(&path).unwrap();
        e.insert("DOWNLOAD_MISSING_FILE".into(), false.into());
        crate::execute("URLDownloader", &mut e).unwrap();
        assert!(!path.exists());
        assert_eq!(e["file_size"].as_unsigned_integer(), Some(4));
        assert_eq!(
            e["file_md5"].as_string(),
            Some("8d777f385d3dfec8815d20f7496026dc")
        );
        assert!(crate::execute("URLDownloader", &mut e).is_err());
        assert_eq!(fs::read_dir(t.path("downloads")).unwrap().count(), 2);
        let requests = server.join().unwrap();
        assert!(requests[1].contains("If-None-Match: \"v1\""));
        assert!(requests[1].contains("If-Modified-Since:"));
        assert!(!requests[2].contains("If-None-Match:"));
    }
    #[test]
    fn prefetch_and_authenticated_python_alias() {
        let t = Temp::new();
        let head="HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=\"../prefetched.pkg\"\r\nContent-Length: 4\r\nConnection: close\r\n\r\n".to_string();
        let (url, server) = server(vec![head, ok()]);
        let mut e = env(&[
            ("url", &url),
            ("download_dir", &t.path("downloads")),
            ("filename", "ignored.pkg"),
        ]);
        e.insert("prefetch_filename".into(), true.into());
        let mut headers = Dictionary::new();
        headers.insert("Authorization".into(), "Basic dXNlcjpwYXNz".into());
        e.insert("request_headers".into(), headers.into());
        crate::execute("URLDownloaderPython", &mut e).unwrap();
        assert_eq!(e["filename"].as_string(), Some("prefetched.pkg"));
        assert!(t.0.join("downloads/prefetched.pkg").exists());
        assert_eq!(e["download_url"].as_string(), Some(url.as_str()));
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("HEAD "));
        assert!(requests
            .iter()
            .all(|r| r.contains("Authorization: Basic dXNlcjpwYXNz")));
    }
    #[test]
    fn filesize_comparison_and_boolean_validation() {
        let mut e = Dictionary::new();
        e.insert("CHECK_FILESIZE_ONLY".into(), "true".into());
        let headers = BTreeMap::from([
            ("content-length".into(), "4".into()),
            ("etag".into(), "new".into()),
        ]);
        assert!(!changed(
            &e,
            Path::new("/does-not-exist"),
            &json!({"http_headers":{"Content-Length":4,"ETag":"old"}}),
            &headers
        )
        .unwrap());
        e.insert("CHECK_FILESIZE_ONLY".into(), "invalid".into());
        assert!(boolean(&e, "CHECK_FILESIZE_ONLY", false).is_err());
    }
}
