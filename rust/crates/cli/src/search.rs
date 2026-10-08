//! Search the pinned-format AutoPkg recipe index using native curl.
use plist::{Dictionary, Value as Plist};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
const ENDPOINT: &str = "https://api.github.com/repos/autopkg/index/contents/v1/index.json?ref=main";
const RAW: &str = "https://raw.githubusercontent.com/autopkg/index/main/v1/index.json";
fn expand(path: &str) -> PathBuf {
    if let Some(tail) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(tail);
        }
    }
    PathBuf::from(path)
}
fn token(prefs: &Dictionary) -> Option<String> {
    let value = prefs
        .get("GITHUB_TOKEN")
        .and_then(Plist::as_string)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            fs::read_to_string(expand(
                prefs
                    .get("GITHUB_TOKEN_PATH")
                    .and_then(Plist::as_string)
                    .unwrap_or("~/.autopkg_gh_token"),
            ))
            .ok()
        })?;
    let value = value.trim();
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        autopkg_platform::text_eprintln!(
            "Ignoring malformed GitHub token; continuing unauthenticated."
        );
        None
    } else {
        Some(value.into())
    }
}
fn fetch(
    url: &str,
    accept: &str,
    token: Option<&str>,
    prefs: &Dictionary,
) -> Result<Vec<u8>, String> {
    let binary = prefs
        .get("CURL_PATH")
        .and_then(Plist::as_string)
        .filter(|s| Path::new(s).is_file())
        .unwrap_or("curl");
    let mut cmd = Command::new(binary);
    cmd.args([
        "--location",
        "--silent",
        "--show-error",
        "--fail",
        "--header",
        "User-Agent: AutoPkg",
        "--header",
        &format!("Accept: {accept}"),
    ]);
    if let Some(token) = token {
        cmd.args(["--header", &format!("Authorization: Bearer {token}")]);
    }
    let output = cmd
        .args(["--url", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "curl failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
fn fallback(path: &Path, reason: &str, raw: &str, prefs: &Dictionary) -> Result<(), String> {
    if path.is_file() {
        autopkg_platform::text_eprintln!("WARNING: {reason}. Using cached version.");
        return Ok(());
    }
    let etag = PathBuf::from(format!("{}.etag", path.display()));
    if !etag.is_file() {
        autopkg_platform::text_println!(
            "GitHub API call failed, attempting download from raw URL..."
        );
        if let Ok(bytes) = fetch(raw, "*/*", None, prefs) {
            fs::write(path, bytes).map_err(|e| e.to_string())?;
            fs::write(
                etag,
                "Search index temporarily sourced from raw GitHub URL.",
            )
            .map_err(|e| e.to_string())?;
            autopkg_platform::text_println!("Successfully downloaded search index from raw URL.");
            return Ok(());
        }
    }
    Err(format!("{reason}, and no cached index available."))
}
fn refresh(path: &Path, endpoint: &str, raw: &str, prefs: &Dictionary) -> Result<(), String> {
    let token = token(prefs);
    let meta = match fetch(
        endpoint,
        "application/vnd.github.v3+json",
        token.as_deref(),
        prefs,
    ) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(v) => v,
            Err(_) => return fallback(path, "Invalid response from GitHub API", raw, prefs),
        },
        Err(_) => return fallback(path, "Unable to check for search index updates", raw, prefs),
    };
    let status = meta["status"]
        .as_u64()
        .or_else(|| meta["status"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    if status >= 400 {
        return fallback(
            path,
            meta["message"]
                .as_str()
                .unwrap_or(&format!("Error {status}")),
            raw,
            prefs,
        );
    }
    let Some(sha) = meta["sha"].as_str().filter(|s| !s.is_empty()) else {
        return fallback(
            path,
            meta["message"]
                .as_str()
                .unwrap_or("Invalid response from GitHub API"),
            raw,
            prefs,
        );
    };
    let size = meta["size"].as_u64().unwrap_or(0);
    if size > 90 * 1024 * 1024 {
        autopkg_platform::text_eprintln!("WARNING: Search index size is {} GitHub's API limit for raw content retrieval (100 MB). Please open an issue here if one was not already created: https://github.com/autopkg/autopkg/issues",if size>100*1024*1024{"greater than"}else{"nearing"});
    }
    let etag = PathBuf::from(format!("{}.etag", path.display()));
    if path.is_file() && fs::read_to_string(&etag).is_ok_and(|s| s.trim_matches('"') == sha) {
        return Ok(());
    }
    if fs::write(&etag, sha).is_err() {
        autopkg_platform::text_eprintln!("ERROR: Unable to save search index cache. Please check permissions at {} and try again.",etag.display());
    }
    autopkg_platform::text_println!("Refreshing local search index...");
    match fetch(
        endpoint,
        "application/vnd.github.v3.raw",
        token.as_deref(),
        prefs,
    ) {
        Ok(bytes) => fs::write(path, bytes).map_err(|e| e.to_string()),
        Err(_) => fallback(path, "Unable to download updated search index", raw, prefs),
    }
}
fn load(root: &Path, endpoint: &str, raw: &str, prefs: &Dictionary) -> Result<Value, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = root.join("search_index.json");
    refresh(&path, endpoint, raw, prefs)?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    match serde_json::from_slice(&bytes) {
        Ok(v) => Ok(v),
        Err(_) => {
            autopkg_platform::text_eprintln!(
                "Invalid search index cache at {}. Deleting and retrying...",
                path.display()
            );
            let _ = fs::remove_file(&path);
            refresh(&path, endpoint, raw, prefs)?;
            Ok(fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_else(|| serde_json::json!({})))
        }
    }
}
fn normalize(keyword: &str) -> String {
    let mut value = keyword.to_lowercase();
    for ext in [".recipe", ".recipe.plist", ".recipe.yaml"] {
        if value.ends_with(ext) {
            value.truncate(value.len() - ext.len());
            break;
        }
    }
    value.replace([' ', '.', ',', '-'], "")
}
#[derive(Debug, PartialEq, Eq)]
struct Row {
    name: String,
    repo: String,
    path: String,
}
fn matches(index: &Value, keyword: &str, path_only: bool) -> Result<Vec<Row>, String> {
    if index.as_object().is_some_and(|o| o.is_empty()) {
        return Ok(vec![]);
    }
    let names = index["shortnames"]
        .as_object()
        .ok_or("Search index shortnames must be a dictionary")?;
    let identifiers = index["identifiers"]
        .as_object()
        .ok_or("Search index identifiers must be a dictionary")?;
    let keyword = normalize(keyword);
    let mut ids = BTreeSet::new();
    for (name, values) in names {
        if normalize(name).contains(&keyword) {
            for id in values
                .as_array()
                .ok_or("Search index shortnames must contain arrays")?
            {
                ids.insert(id.as_str().ok_or("Search identifier must be a string")?);
            }
        }
    }
    for (id, info) in identifiers {
        for key in if path_only {
            &["path"][..]
        } else {
            &["name", "app_display_name"][..]
        } {
            if info[*key]
                .as_str()
                .is_some_and(|v| normalize(v).contains(&keyword))
            {
                ids.insert(id);
            }
        }
    }
    ids.into_iter()
        .map(|id| {
            let item = identifiers
                .get(id)
                .ok_or("Search index references missing identifier")?;
            let path = item["path"].as_str().ok_or("Search result requires path")?;
            let repo = item["repo"].as_str().ok_or("Search result requires repo")?;
            Ok(Row {
                name: path.rsplit('/').next().unwrap_or(path).into(),
                repo: if repo.starts_with("autopkg/") {
                    repo.replace("autopkg/", "")
                } else {
                    repo.into()
                },
                path: path.into(),
            })
        })
        .collect()
}
fn print_rows(rows: &[Row]) {
    if rows.is_empty() {
        autopkg_platform::text_eprintln!("Nothing found.");
        return;
    }
    let mut limited: Vec<_> = rows.iter().take(100).collect();
    let mut widths = [4, 4, 4];
    for row in &limited {
        for (i, cell) in [&row.name, &row.repo, &row.path].iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count())
        }
    }
    let render = |cells: [&str; 3], separator: bool| {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            line.push_str(if separator { "" } else { cell });
            if separator {
                line.push_str(&"-".repeat(cell.chars().count()));
            }
            line.push_str(&" ".repeat(widths[i] - cell.chars().count() + 4));
        }
        autopkg_platform::text_println!("{line}");
    };
    autopkg_platform::text_println!();
    render(["Name", "Repo", "Path"], false);
    render(["Name", "Repo", "Path"], true);
    limited.sort_by_key(|r| r.repo.to_lowercase());
    for row in limited {
        render([&row.name, &row.repo, &row.path], false)
    }
    autopkg_platform::text_println!("\nTo add a new recipe repo, use `autopkg repo-add <repo name>`\n\nIf you don't see the recipe you're looking for, try searching https://autopkgweb.com/ (maintained by @jannheider).");
    if rows.len() > 100 {
        autopkg_platform::text_eprintln!("\nWARNING: Only showing first 100 out of {} total results. Please try a more specific search term.",rows.len());
    }
}
fn quote(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'~' => {
                (*b as char).to_string()
            }
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
pub fn run(args: &[String]) -> Result<i32, String> {
    let mut prefs = None;
    let mut path_only = false;
    let mut user = "autopkg";
    let mut query = None;
    let mut use_token = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--prefs" => prefs = Some(iter.next().ok_or("--prefs requires a path")?.as_str()),
            "-p" | "--path-only" => path_only = true,
            "-u" | "--user" => user = iter.next().ok_or("--user requires a name")?,
            "-t" | "--use-token" => use_token = true,
            "-h" | "--help" => {
                autopkg_platform::text_println!(
                    "Usage: russet search [--path-only] [--user ORG] search_term"
                );
                return Ok(0);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unsupported search option '{flag}'"))
            }
            _ => {
                if query.is_none() {
                    query = Some(arg.as_str())
                }
            }
        }
    }
    let query = query
        .filter(|s| !s.trim().is_empty())
        .ok_or("ERROR: No search query specified!")?;
    if use_token {
        autopkg_platform::text_eprintln!(
            "WARNING: Deprecated option '--use-token' provided, ignoring."
        );
    }
    if user != "autopkg" {
        if !user.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            autopkg_platform::text_eprintln!(
                "WARNING: GitHub user/org names contain only alphanumeric characters and dashes."
            );
        }
        let user: String = user
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        autopkg_platform::text_println!("'autopkg search' no longer directly searches GitHub users or orgs other than the autopkg org.\nHowever, this page may provide some useful results:\nhttps://github.com/search?q={}+org%3A{user}+lang%3Axml+OR+lang%3Ayaml&type=code",quote(query).to_lowercase());
        return Ok(0);
    }
    let prefs = super::manage::load_preferences(prefs)?;
    let root = super::cache::root(&prefs)?;
    print_rows(&matches(
        &load(&root, ENDPOINT, RAW, &prefs)?,
        query,
        path_only,
    )?);
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    #[test]
    fn normalized_matching_keeps_shortnames_with_path_only() {
        let index = json!({"shortnames":{"Foo-Bar.recipe":["a"]},"identifiers":{"a":{"name":"Foo Bar","repo":"autopkg/recipes","path":"Apps/Foo.recipe"},"b":{"app_display_name":"Foo-Bar","repo":"other/repo","path":"Other/X.recipe"}}});
        assert_eq!(
            matches(&index, "foo bar.recipe.yaml", false).unwrap().len(),
            2
        );
        assert_eq!(matches(&index, "foo bar", true).unwrap().len(), 1);
        assert_eq!(
            matches(&index, "other", true).unwrap()[0].repo,
            "other/repo"
        );
        assert_eq!(quote("Foo App/"), "Foo+App%2F");
    }
    #[test]
    fn local_index_refresh_etag_and_offline_cache() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/index", listener.local_addr().unwrap());
        let index = json!({"shortnames":{},"identifiers":{}}).to_string();
        let server = thread::spawn(move || {
            let mut requests = vec![];
            for body in [
                json!({"sha":"abc","size":1}).to_string(),
                index,
                json!({"sha":"abc"}).to_string(),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut bytes = vec![0; 8192];
                let n = socket.read(&mut bytes).unwrap();
                requests.push(String::from_utf8_lossy(&bytes[..n]).into_owned());
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let temp = tempfile::tempdir().unwrap();
        let mut prefs = Dictionary::new();
        prefs.insert("GITHUB_TOKEN".into(), "test-token".into());
        let first = load(temp.path(), &endpoint, &endpoint, &prefs).unwrap();
        assert!(first["identifiers"].is_object());
        load(temp.path(), &endpoint, &endpoint, &prefs).unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].contains("Authorization: Bearer test-token"));
        assert!(requests[1].contains("application/vnd.github.v3.raw"));
        load(temp.path(), &endpoint, &endpoint, &prefs).unwrap();
        assert_eq!(
            fs::read_to_string(temp.path().join("search_index.json.etag")).unwrap(),
            "abc"
        );
    }
    #[test]
    fn uncached_api_failure_downloads_raw_index() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/api", listener.local_addr().unwrap());
        let raw = endpoint.replace("/api", "/raw");
        let server = thread::spawn(move || {
            let mut requests = vec![];
            for body in ["not-json", r#"{"shortnames":{},"identifiers":{}}"#] {
                let (mut socket, _) = listener.accept().unwrap();
                let mut bytes = [0; 8192];
                let size = socket.read(&mut bytes).unwrap();
                requests.push(String::from_utf8_lossy(&bytes[..size]).into_owned());
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });
        let temp = tempfile::tempdir().unwrap();
        let mut prefs = Dictionary::new();
        prefs.insert(
            "GITHUB_TOKEN_PATH".into(),
            temp.path()
                .join("no-token")
                .to_string_lossy()
                .into_owned()
                .into(),
        );
        assert!(load(temp.path(), &endpoint, &raw, &prefs).unwrap()["identifiers"].is_object());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /api"));
        assert!(requests[1].starts_with("GET /raw"));
        assert_eq!(
            fs::read_to_string(temp.path().join("search_index.json.etag")).unwrap(),
            "Search index temporarily sourced from raw GitHub URL."
        );
    }
    #[test]
    fn malformed_token_and_uncached_error() {
        let mut prefs = Dictionary::new();
        prefs.insert("GITHUB_TOKEN".into(), "bad\nheader".into());
        assert!(token(&prefs).is_none());
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("missing.etag"), "previous").unwrap();
        assert!(fallback(
            &temp.path().join("missing"),
            "failure",
            "http://127.0.0.1:1",
            &prefs
        )
        .unwrap_err()
        .contains("no cached index"));
    }
}
