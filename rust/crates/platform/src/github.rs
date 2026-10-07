//! GitHub Releases API metadata with sequential pagination and token fallback.
use fancy_regex::Regex;
use plist::{Dictionary, Value};
use serde_json::Value as Json;
use std::{cmp::Ordering, env, fs, path::PathBuf, process::Command};

fn truth(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Boolean(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Integer(i)) => i.as_signed().map(|n| n != 0).unwrap_or(true),
        Some(Value::Real(f)) => *f != 0.0,
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Dictionary(d)) => !d.is_empty(),
        Some(Value::Data(d)) => !d.is_empty(),
        _ => true,
    }
}
fn string<'a>(env: &'a Dictionary, key: &str, default: &'a str) -> Result<&'a str, String> {
    match env.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_string()
            .ok_or_else(|| format!("{key} must be a string")),
    }
}
fn expanded(path: &str) -> PathBuf {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(""));
        }
    }
    PathBuf::from(path)
}
fn sanitized(token: &str, source: &str) -> Option<String> {
    let token = token.trim();
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        crate::text_eprintln!(
            "Ignoring malformed GitHub token from {source}; continuing unauthenticated."
        );
        None
    } else {
        Some(token.into())
    }
}
fn token(env: &Dictionary, preferences: Option<&Dictionary>) -> Result<Option<String>, String> {
    let native = if preferences.is_none() {
        crate::preference("com.github.autopkg", "GITHUB_TOKEN")?
    } else {
        None
    };
    if let Some(value) = preferences
        .and_then(|prefs| prefs.get("GITHUB_TOKEN"))
        .or(native.as_ref())
        .filter(|v| truth(Some(v)))
    {
        return Ok(sanitized(
            value.as_string().ok_or("GITHUB_TOKEN must be a string")?,
            "GITHUB_TOKEN preference",
        ));
    }
    let path = expanded(string(env, "GITHUB_TOKEN_PATH", "~/.autopkg_gh_token")?);
    if !path.exists() {
        return Ok(None);
    }
    match fs::read_to_string(&path) {
        Ok(value) => Ok(sanitized(&value, &path.to_string_lossy())),
        Err(error) => {
            crate::text_eprintln!(
                "Couldn't read token file at {}! Error: {error}",
                path.display()
            );
            Ok(None)
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Component {
    Number(String),
    Text(String),
}
// Unicode 16 decimal blocks, matching Python 3.14's int() conversion of Nd digits.
fn decimal_digit(ch: char) -> Option<char> {
    const ZEROS: &[u32] = &[
        0x30, 0x660, 0x6f0, 0x7c0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66,
        0xde6, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80, 0x1a90,
        0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
        0xff10, 0x104a0, 0x10d30, 0x10d40, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450,
        0x114d0, 0x11650, 0x116c0, 0x116d0, 0x116da, 0x11730, 0x118e0, 0x11950, 0x11bf0, 0x11c50,
        0x11d50, 0x11da0, 0x11f50, 0x16130, 0x16a60, 0x16ac0, 0x16b50, 0x16d70, 0x1ccf0, 0x1d7ce,
        0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0, 0x1e4f0, 0x1e5f1, 0x1e950, 0x1fbf0,
    ];
    let point = ch as u32;
    ZEROS.iter().find_map(|zero| {
        (point >= *zero && point <= zero + 9).then(|| (b'0' + (point - zero) as u8) as char)
    })
}
fn components(version: &str) -> Vec<Component> {
    // Capturing split preserves unmatched punctuation and uppercase runs, just
    // like distutils LooseVersion. Numbers stay decimal strings to avoid overflow.
    let re = regex::Regex::new(r"(\d+|[a-z]+|\.)").unwrap();
    let mut result = Vec::new();
    let mut offset = 0;
    let mut push = |text: &str| {
        if text.is_empty() || text == "." {
            return;
        }
        if let Some(decimal) = text.chars().map(decimal_digit).collect::<Option<String>>() {
            let digits = decimal.trim_start_matches('0');
            result.push(Component::Number(
                if digits.is_empty() { "0" } else { digits }.into(),
            ));
        } else {
            result.push(Component::Text(text.into()));
        }
    };
    for matched in re.find_iter(version) {
        push(&version[offset..matched.start()]);
        push(matched.as_str());
        offset = matched.end();
    }
    push(&version[offset..]);
    result
}
/// AutoPkg's padded LooseVersion order: integer components sort before strings.
/// Numeric components are unbounded, so long build numbers do not overflow.
pub fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = components(left);
    let right = components(right);
    let zero = Component::Number("0".into());
    for index in 0..left.len().max(right.len()) {
        let order = match (
            left.get(index).unwrap_or(&zero),
            right.get(index).unwrap_or(&zero),
        ) {
            (Component::Number(a), Component::Number(b)) => {
                a.len().cmp(&b.len()).then_with(|| a.cmp(b))
            }
            (Component::Text(a), Component::Text(b)) => a.cmp(b),
            (Component::Number(_), Component::Text(_)) => Ordering::Less,
            (Component::Text(_), Component::Number(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

fn request(env: &Dictionary, url: &str, auth: Option<&str>) -> Result<(Option<Json>, u16), String> {
    let content = tempfile::NamedTempFile::new()
        .map_err(|e| e.to_string())?
        .into_temp_path();
    let curl = crate::downloads::curl_binary(env)?;
    let mut command = Command::new(curl);
    command.args([
        "--location",
        "--silent",
        "--show-error",
        "--dump-header",
        "-",
        "-X",
        "GET",
        "--header",
        "User-Agent: AutoPkg",
        "--header",
        "Accept: application/vnd.github.v3+json",
    ]);
    if let Some(auth) = auth {
        command.args(["--header", &format!("Authorization: token {auth}")]);
    }
    if let Some(options) = env.get("curl_opts") {
        for option in options.as_array().ok_or("curl_opts must be an array")? {
            command.arg(
                option
                    .as_string()
                    .ok_or("curl_opts entries must be strings")?,
            );
        }
    }
    let output = command
        .args(["--url", url, "--output"])
        .arg(&content)
        .output()
        .map_err(|e| format!("Cannot execute curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let status = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with("HTTP/"))
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u16>().ok())
        .next_back()
        .unwrap_or(0);
    // The reference treats an invalid/empty JSON body as no data, then reports
    // the HTTP status or the no-releases error at the processor boundary.
    let data = fs::read(&content).map_err(|e| e.to_string())?;
    Ok((serde_json::from_slice(&data).ok(), status))
}
fn call_api(
    env: &Dictionary,
    url: &str,
    auth: Option<&str>,
) -> Result<(Option<Json>, u16), String> {
    let (data, status) = request(env, url, auth)?;
    if auth.is_some() && status == 401 {
        crate::text_eprintln!("WARNING: Your GitHub token appears invalid or expired. Regenerate it at https://github.com/settings/tokens. Continuing without it.");
        request(env, url, None)
    } else {
        Ok((data, status))
    }
}
fn selected<'a>(
    releases: &'a [Json],
    pattern: Option<&str>,
    prereleases: bool,
) -> Result<Option<(&'a Json, &'a Json)>, String> {
    let mut regex = None;
    for release in releases {
        if release
            .get("prerelease")
            .and_then(Json::as_bool)
            .ok_or("GitHub release is missing prerelease")?
            && !prereleases
        {
            continue;
        }
        let Some(assets) = release.get("assets").and_then(Json::as_array) else {
            continue;
        };
        for asset in assets {
            let Some(pattern) = pattern.filter(|s| !s.is_empty()) else {
                return Ok(Some((release, asset)));
            };
            if regex.is_none() {
                regex = Some(Regex::new(pattern).map_err(|e| format!("Invalid regex: {e}"))?);
            }
            let name = asset
                .get("name")
                .and_then(Json::as_str)
                .ok_or("GitHub asset is missing name")?;
            if regex
                .as_ref()
                .unwrap()
                .find(name)
                .map_err(|e| format!("Invalid regex: {e}"))?
                .is_some_and(|m| m.start() == 0)
            {
                crate::processor_output(
                    1,
                    format!(
                        "Matched regex '{pattern}' among asset(s): {}",
                        assets
                            .iter()
                            .filter_map(|a| a["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
                return Ok(Some((release, asset)));
            }
        }
    }
    Ok(None)
}
fn record(env: &mut Dictionary, release: &Json, asset: &Json) -> Result<(), String> {
    for (output, input) in [
        ("url", "browser_download_url"),
        ("asset_url", "url"),
        ("asset_created_at", "created_at"),
    ] {
        env.insert(
            output.into(),
            asset
                .get(input)
                .and_then(Json::as_str)
                .ok_or_else(|| format!("GitHub asset is missing {input}"))?
                .into(),
        );
    }
    let tag = release
        .get("tag_name")
        .and_then(Json::as_str)
        .ok_or("GitHub release is missing tag_name")?;
    let version = if tag.starts_with('v') {
        tag.trim_start_matches(['v', '.'])
    } else {
        tag
    };
    env.insert("version".into(), version.into());
    env.insert(
        "release_notes".into(),
        release
            .get("body")
            .and_then(Json::as_str)
            .unwrap_or("")
            .into(),
    );
    Ok(())
}
pub fn execute(env: &mut Dictionary) -> Result<(), String> {
    execute_with_preferences(env, None)
}
pub fn execute_with_preferences(
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
) -> Result<(), String> {
    let repo = string(env, "github_repo", "")?.to_owned();
    if !env.contains_key("github_repo") {
        return Err("GitHubReleasesInfoProvider requires github_repo".into());
    }
    let base = string(env, "GITHUB_URL", "https://api.github.com")?.to_owned();
    let latest = truth(env.get("latest_only"));
    let per_page = match env.get("GITHUB_RELEASES_PER_PAGE") {
        None => "30".into(),
        Some(Value::Integer(i)) => i.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => return Err("GITHUB_RELEASES_PER_PAGE must be an integer or string".into()),
    };
    let pattern = env
        .get("asset_regex")
        .map(|v| v.as_string().ok_or("asset_regex must be a string"))
        .transpose()?
        .map(str::to_owned);
    let prereleases = truth(env.get("include_prereleases"));
    let sort = truth(env.get("sort_by_highest_tag_names"));
    let mut page = 1u64;
    loop {
        crate::processor_output(1, format!("Fetching page {page} of GitHub releases"));
        let suffix = if latest {
            "/latest".into()
        } else {
            format!("?page={page}&per_page={per_page}")
        };
        let url = format!("{base}/repos/{repo}/releases{suffix}");
        // The Python processor constructs a fresh GitHubSession for every page.
        let auth = token(env, preferences)?;
        let (data, status) = call_api(env, &url, auth.as_deref())?;
        if status != 200 {
            return Err(format!("Unexpected GitHub API status code {status}."));
        }
        let mut releases = match data {
            Some(Json::Object(object)) if latest && !object.is_empty() => {
                vec![Json::Object(object)]
            }
            Some(Json::Array(array)) if !latest && !array.is_empty() => array,
            _ => {
                return Err(format!(
                    "No releases found for repo '{repo}' on page {page}"
                ))
            }
        };
        if sort {
            for release in &releases {
                if release.get("tag_name").and_then(Json::as_str).is_none() {
                    return Err("GitHub release is missing tag_name".into());
                }
            }
            releases.sort_by(|a, b| {
                compare_versions(
                    b["tag_name"].as_str().unwrap(),
                    a["tag_name"].as_str().unwrap(),
                )
            });
        }
        if let Some((release, asset)) = selected(&releases, pattern.as_deref(), prereleases)? {
            crate::processor_output(
                1,
                format!(
                    "Selected asset '{}' from release '{}'",
                    asset["name"].as_str().unwrap_or(""),
                    release["name"].as_str().unwrap_or("")
                ),
            );
            return record(env, release, asset);
        }
        if latest {
            return Err("No release assets were found that satisfy the criteria.".into());
        }
        crate::processor_output(1, format!("No releases found on page {page}"));
        page = page.checked_add(1).ok_or("GitHub pagination overflow")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    fn release(tag: &str, name: &str, prerelease: bool) -> Json {
        json!({"name":tag,"tag_name":tag,"prerelease":prerelease,"body":null,"assets":[{"name":name,"browser_download_url":"https://example.invalid/download","url":"https://api.example.invalid/asset","created_at":"2026-01-01T00:00:00Z"}]})
    }
    fn server(
        responses: Vec<(u16, String)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            for (status, body) in responses {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                Instant::now() < deadline,
                                "Expected HTTP request did not arrive"
                            );
                            thread::sleep(Duration::from_millis(10));
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
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let mut bytes = [0u8; 1024];
                while !data.windows(4).any(|w| w == b"\r\n\r\n") {
                    let count = stream.read(&mut bytes).unwrap();
                    assert!(count > 0);
                    data.extend_from_slice(&bytes[..count]);
                }
                observed
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(data).unwrap());
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (address, requests, handle)
    }
    fn environment(base: String) -> Dictionary {
        Dictionary::from_iter([
            ("github_repo", "owner/project".into()),
            ("GITHUB_URL", base.into()),
            ("GITHUB_TOKEN", Value::String("test-token".into())),
            (
                "curl_opts",
                Value::Array(vec!["--noproxy".into(), "*".into()]),
            ),
        ])
    }
    #[test]
    fn loose_versions_pad_and_sort_without_numeric_overflow() {
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.2", "1.10"), Ordering::Less);
        assert_eq!(compare_versions("1.0", "1.0rc1"), Ordering::Less);
        assert_eq!(compare_versions("1.0a", "1.0b"), Ordering::Less);
        assert_eq!(compare_versions("v1.99", "v2.0"), Ordering::Less);
        assert_eq!(
            compare_versions(
                "999999999999999999999999999999",
                "1000000000000000000000000000000"
            ),
            Ordering::Less
        );
        assert_eq!(compare_versions("01.002", "1.2"), Ordering::Equal);
        assert_eq!(compare_versions("١.٢", "1.2"), Ordering::Equal);
        assert_eq!(compare_versions("𝟙.𝟚", "1.2"), Ordering::Equal);
    }
    #[test]
    fn asset_regex_is_anchored_and_supports_lookaround() {
        let releases = vec![
            release("v3", "wanted.pkg", true),
            release("v2", "not-wanted.pkg", false),
            release("v1", "wanted.pkg", false),
        ];
        let (rel, _) = selected(&releases, Some(r"wanted(?=\.pkg)"), false)
            .unwrap()
            .unwrap();
        assert_eq!(rel["tag_name"], "v1");
        assert_eq!(
            selected(&releases, None, true).unwrap().unwrap().0["tag_name"],
            "v3"
        );
        assert!(selected(&releases, Some("["), true).is_err());
    }
    #[test]
    fn preference_context_is_separate_from_recipe_token() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("token");
        let env = Dictionary::from_iter([
            ("GITHUB_TOKEN", Value::String("recipe-token".into())),
            (
                "GITHUB_TOKEN_PATH",
                path.to_string_lossy().into_owned().into(),
            ),
        ]);
        let preferences = Dictionary::new();
        assert_eq!(token(&env, Some(&preferences)).unwrap(), None);
        fs::write(&path, "file-token\n").unwrap();
        assert_eq!(
            token(&env, Some(&preferences)).unwrap().as_deref(),
            Some("file-token")
        );
        let preferences =
            Dictionary::from_iter([("GITHUB_TOKEN", Value::String("preference-token".into()))]);
        assert_eq!(
            token(&env, Some(&preferences)).unwrap().as_deref(),
            Some("preference-token")
        );
        assert_eq!(env["GITHUB_TOKEN"].as_string(), Some("recipe-token"));
    }
    #[test]
    fn pagination_retries_bad_token_only_for_the_failed_request() {
        let no_asset = json!([{"tag_name":"v9","prerelease":false,"assets":[]}]);
        let final_page = json!([release("vv.1.2", "app.pkg", false)]);
        let (base, requests, worker) = server(vec![
            (401, "{}".into()),
            (200, no_asset.to_string()),
            (200, final_page.to_string()),
        ]);
        let mut env = environment(base);
        env.insert("GITHUB_RELEASES_PER_PAGE".into(), 2.into());
        execute_with_preferences(
            &mut env,
            Some(&Dictionary::from_iter([(
                "GITHUB_TOKEN",
                Value::String("test-token".into()),
            )])),
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(env["version"].as_string(), Some("1.2"));
        assert_eq!(env["release_notes"].as_string(), Some(""));
        let requests = requests.lock().unwrap();
        assert!(requests[0].contains("GET /repos/owner/project/releases?page=1&per_page=2"));
        assert!(requests[0].contains("Authorization: token test-token"));
        assert!(!requests[1].contains("Authorization:"));
        assert!(requests[2].contains("page=2&per_page=2"));
        assert!(requests[2].contains("Authorization: token test-token"));
        assert!(requests[0].contains("User-Agent: AutoPkg"));
        assert!(requests[0].contains("Accept: application/vnd.github.v3+json"));
    }
    #[test]
    fn latest_endpoint_does_not_paginate_missing_assets() {
        let item = release("v2", "app.zip", false);
        let (base, requests, worker) = server(vec![(200, item.to_string())]);
        let mut env = environment(base);
        env.insert("latest_only".into(), true.into());
        env.insert("asset_regex".into(), r".*\.pkg$".into());
        assert_eq!(
            execute_with_preferences(
                &mut env,
                Some(&Dictionary::from_iter([(
                    "GITHUB_TOKEN",
                    Value::String("test-token".into())
                )]))
            )
            .unwrap_err(),
            "No release assets were found that satisfy the criteria."
        );
        worker.join().unwrap();
        assert!(requests.lock().unwrap()[0].contains("/releases/latest "));
    }
    #[test]
    fn highest_tags_sort_within_each_page() {
        let items = json!([
            release("v1.9", "old.pkg", false),
            release("v1.10", "new.pkg", false)
        ]);
        let (base, _, worker) = server(vec![(200, items.to_string())]);
        let mut env = environment(base);
        env.insert("sort_by_highest_tag_names".into(), true.into());
        execute_with_preferences(
            &mut env,
            Some(&Dictionary::from_iter([(
                "GITHUB_TOKEN",
                Value::String("test-token".into()),
            )])),
        )
        .unwrap();
        worker.join().unwrap();
        assert_eq!(env["version"].as_string(), Some("1.10"));
    }
    #[test]
    fn http_failures_and_empty_pages_are_explicit() {
        for (status, body, expected) in [
            (403, "{}", "Unexpected GitHub API status code 403."),
            (
                200,
                "[]",
                "No releases found for repo 'owner/project' on page 1",
            ),
            (
                200,
                "invalid json",
                "No releases found for repo 'owner/project' on page 1",
            ),
        ] {
            let (base, _, worker) = server(vec![(status, body.into())]);
            let mut env = environment(base);
            assert_eq!(
                execute_with_preferences(
                    &mut env,
                    Some(&Dictionary::from_iter([(
                        "GITHUB_TOKEN",
                        Value::String("test-token".into())
                    )]))
                )
                .unwrap_err(),
                expected
            );
            worker.join().unwrap();
        }
        assert!(sanitized(" bad token ", "test").is_none());
        assert_eq!(sanitized(" token\n", "test").as_deref(), Some("token"));
    }
}
