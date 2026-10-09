//! Differential tests: each scenario runs through a processor with curl and
//! with the native engine, and every observable result must match: the
//! processor's outputs and errors, cached files and metadata, and the
//! requests the server received.
use super::super::PREFERENCE;
use super::fixture::{serve, Reply, Request, Server};
use plist::{Dictionary, Value};

struct Scenario {
    servers: Vec<Server>,
    path: &'static str,
}

fn observe(
    processor: &str,
    inputs: &Dictionary,
    runs: usize,
    native: bool,
    scenario: Scenario,
) -> String {
    let directory = tempfile::tempdir().unwrap();
    let downloads = directory.path().join("downloads");
    let mut env = inputs.clone();
    env.insert(
        "url".into(),
        format!("{}{}", scenario.servers[0].base, scenario.path).into(),
    );
    env.insert(
        "download_dir".into(),
        downloads.to_string_lossy().into_owned().into(),
    );
    env.insert(PREFERENCE.into(), native.into());
    let mut record = String::new();
    for run in 0..runs {
        let result = crate::execute_standalone(processor, &mut env);
        let mut shown = env.clone();
        shown.remove(PREFERENCE);
        record.push_str(&format!(
            "run {run}: {:?}\n{}\n",
            result.map_err(|e| (e.kind, e.message)),
            plist::python_repr(&Value::Dictionary(shown))
        ));
    }
    let mut files = std::fs::read_dir(&downloads)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("tmp") {
                        "retained temporary file".to_string()
                    } else {
                        format!(
                            "{name}: {:?}",
                            String::from_utf8_lossy(
                                &std::fs::read(entry.path()).unwrap_or_default()
                            )
                        )
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort();
    record.push_str(&format!("files: {files:?}\n"));
    for (index, server) in scenario.servers.iter().enumerate() {
        for request in server.requests() {
            record.push_str(&format!("server{index}: {}\n", request.normalized()));
        }
    }
    for (index, server) in scenario.servers.iter().enumerate() {
        record = record.replace(server.authority(), &format!("server{index}"));
    }
    // Property-list representations escape backslashes in Windows paths.
    let directory = directory.path().to_string_lossy().into_owned();
    record
        .replace(&directory.replace('\\', "\\\\"), "<dir>")
        .replace(&directory, "<dir>")
}

/// Runs a scenario on both backends and requires identical observations.
fn differential(
    processors: &[&str],
    inputs: &[(&str, Value)],
    runs: usize,
    setup: impl Fn() -> Scenario,
) {
    if !super::curl_available() {
        return;
    }
    let inputs = inputs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect::<Dictionary>();
    for processor in processors {
        let curl = observe(processor, &inputs, runs, false, setup());
        let native = observe(processor, &inputs, runs, true, setup());
        if curl != native {
            let differences = curl
                .lines()
                .zip(native.lines())
                .filter(|(a, b)| a != b)
                .map(|(a, b)| format!("curl:   {a}\nnative: {b}"))
                .collect::<Vec<_>>();
            panic!("{processor} differs:\n{}", differences.join("\n"));
        }
    }
}

const BOTH: [&str; 2] = ["URLDownloader", "URLDownloaderPython"];

fn cached(request: &Request) -> Reply {
    if request.header("If-None-Match") == Some("\"v1\"") {
        return Reply::new("304 Not Modified").header("ETag", "\"v1\"");
    }
    Reply::new("200 Fine")
        .header("ETag", "\"v1\"")
        .header("Last-Modified", "Thu, 01 Oct 2026 00:00:00 GMT")
        .header("X-Duplicate", "first")
        .header("x-duplicate", "second")
        .body("package bytes")
}

fn headers(pairs: &[(&str, &str)]) -> Value {
    Value::Dictionary(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
            .collect(),
    )
}

#[test]
fn caching_conditional_requests_and_header_capture_match_curl() {
    differential(
        &BOTH,
        &[
            ("COMPUTE_HASHES", true.into()),
            (
                "request_headers",
                headers(&[
                    ("X-Token", "abc"),
                    ("X-Removed", ""),
                    ("User-Agent", "Recipe/1.0"),
                ]),
            ),
        ],
        2,
        || Scenario {
            servers: vec![serve(cached, None)],
            path: "/app.pkg",
        },
    );
}

#[test]
fn redirects_match_curl_including_credentials_across_origins() {
    differential(
        &BOTH,
        &[(
            "request_headers",
            headers(&[
                ("Authorization", "Bearer secret"),
                ("Cookie", "session=1"),
                ("X-Kept", "yes"),
            ]),
        )],
        1,
        || {
            let other = serve(|_| Reply::new("200 OK").body("moved"), None);
            let target = format!("{}/final/App.pkg", other.base);
            let origin = serve(
                move |request| match request.target.as_str() {
                    "/start" => Reply::new("302 Found").header("Location", "/second?x=1"),
                    _ => Reply::new("301 Moved Permanently").header("Location", &target),
                },
                None,
            );
            Scenario {
                servers: vec![origin, other],
                path: "/start",
            }
        },
    );
}

#[test]
fn http_errors_match_curl() {
    for status in ["404 Not Found", "500 Internal Server Error", "403 Nope"] {
        differential(&BOTH, &[], 1, move || Scenario {
            servers: vec![serve(move |_| Reply::new(status).body("error page"), None)],
            path: "/missing.pkg",
        });
    }
}

#[test]
fn short_bodies_match_curl() {
    differential(&BOTH, &[], 1, || Scenario {
        servers: vec![serve(
            |_| {
                Reply::new("200 OK")
                    .header("Content-Length", 10)
                    .raw_body("abcd")
            },
            None,
        )],
        path: "/short.pkg",
    });
}

#[test]
fn prefetched_filenames_match_curl() {
    differential(&BOTH, &[("prefetch_filename", true.into())], 1, || {
        Scenario {
            servers: vec![serve(
                |request| {
                    Reply::new("200 OK")
                        .header(
                            "Content-Disposition",
                            "attachment; filename=\"../Named App 1.0.pkg\"",
                        )
                        .body(request.method.as_bytes().to_vec())
                },
                None,
            )],
            path: "/download?id=7",
        }
    });
    differential(
        &["URLDownloader"],
        &[("prefetch_filename", true.into())],
        1,
        || {
            let origin = serve(
                |request| match request.target.as_str() {
                    "/latest" => Reply::new("302 Found").header("Location", "/files/Real-2.0.dmg"),
                    _ => Reply::new("200 OK").body("dmg"),
                },
                None,
            );
            Scenario {
                servers: vec![origin],
                path: "/latest",
            }
        },
    );
}

#[test]
fn verified_curl_options_run_natively_and_match_curl() {
    let options = Value::Array(
        [
            "-sSL",
            "--user-agent",
            "Vendor Agent/2.0",
            "-H",
            "X-Option: 1",
            "--referer",
            "https://example.com/start",
            "-f",
        ]
        .into_iter()
        .map(|s| Value::String(s.into()))
        .collect(),
    );
    let mut env = Dictionary::new();
    env.insert("url".into(), "http://127.0.0.1:1/app.pkg".into());
    env.insert("curl_opts".into(), options.clone());
    let (command, _) = crate::downloader::command(
        &env,
        false,
        &["--fail".into(), "--output".into(), "/x".into()],
    )
    .unwrap();
    super::super::options::inspect(&command, false).unwrap();
    differential(&["URLDownloader"], &[("curl_opts", options)], 2, || {
        Scenario {
            servers: vec![serve(cached, None)],
            path: "/app.pkg",
        }
    });
}

/// Header names keep their case with curl, and the native engine sends them
/// in title case, which shows which backend handled the request.
#[test]
fn use_russet_downloader_false_runs_curl() {
    if !super::curl_available() {
        return;
    }
    for (native, expected) in [(false, "x-probe"), (true, "X-Probe")] {
        let server = serve(|_| Reply::new("200 OK").body("x"), None);
        let directory = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([
            (
                "url".to_string(),
                Value::String(format!("{}/a.pkg", server.base)),
            ),
            (
                "download_dir".to_string(),
                directory.path().to_string_lossy().into_owned().into(),
            ),
            (PREFERENCE.to_string(), Value::String(native.to_string())),
            ("request_headers".to_string(), headers(&[("x-probe", "1")])),
        ]);
        crate::execute_standalone("URLDownloader", &mut env).unwrap();
        let requests = server.requests();
        assert!(
            requests[0].headers.iter().any(|(n, _)| n == expected),
            "{native}: {:?}",
            requests[0].headers
        );
    }
}
