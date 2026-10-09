//! Backend selection: which invocations the native engine admits, and the
//! preferences that keep every request on curl.
use super::super::options::{inspect, UserAgent};
use super::super::{policy, PREFERENCE};
use plist::{Dictionary, Value};
use std::process::Command;

fn generated(extra: &[&str]) -> Command {
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--no-buffer",
        "--dump-header",
        "-",
        "--speed-time",
        "30",
        "--location",
        "--url",
        "https://example.com/app.pkg",
        "--fail",
        "--output",
        "/tmp/download",
    ]);
    command.args(extra);
    // The macOS downloader always gives curl a bundle; mirror that so the
    // decision does not depend on the test host.
    command.env("SSL_CERT_FILE", "/tmp/bundle.pem");
    command
}

fn reason(extra: &[&str]) -> String {
    inspect(&generated(extra), false).unwrap_err()
}

#[test]
fn admits_generated_arguments_and_verified_recipe_options() {
    let request = inspect(&generated(&[]), false).unwrap();
    assert!(request.follow && request.fail && !request.head && request.chunkable);
    assert_eq!(request.user_agent, UserAgent::Curl);
    assert_eq!(
        request.low_speed_time,
        Some(std::time::Duration::from_secs(30))
    );
    let request = inspect(
        &generated(&[
            "-sSL",
            "-AAgent/2.0",
            "--header",
            "X-One: 1",
            "-H",
            "X-Removed:",
            "--referer",
            "https://example.com/",
            "-f",
            "--location",
        ]),
        false,
    )
    .unwrap();
    assert!(matches!(request.user_agent, UserAgent::Custom(ref v) if v == "Agent/2.0"));
    let names = request
        .headers
        .iter()
        .map(|(n, v)| format!("{n}={}", v.to_str().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(names, ["referer=https://example.com/", "x-one=1"]);
    // A recipe User-Agent header replaces --user-agent in either order.
    let request = inspect(&generated(&["-H", "User-Agent: A", "-A", "B"]), false).unwrap();
    assert_eq!(request.user_agent, UserAgent::Omitted);
    assert_eq!(request.headers.len(), 1);
    // A user-supplied precondition keeps the transfer in one stream.
    assert!(
        !inspect(&generated(&["-H", "If-Match: \"x\""]), false)
            .unwrap()
            .chunkable
    );
}

#[test]
fn unverified_invocations_stay_on_curl_with_a_redacted_reason() {
    for (extra, expected) in [
        (&["--cookie", "session=secret"][..], "--cookie"),
        (&["--compressed"], "--compressed"),
        (&["-b", "session=secret"], "-b"),
        (&["--user-agent=Agent"], "(unprintable)"),
        (&["https://example.com/other"], "positional"),
        (&["--url", "https://example.com/other"], "one URL"),
        (&["-H", "@headers.txt"], "file"),
        (&["-H", "Host: example.net"], "host"),
        (&["-H", "Range: bytes=0-1"], "range"),
        (&["-H", "Accept-Encoding: gzip"], "accept-encoding"),
        (&["-H", "Accept:"], "accept"),
        (&["-H", "NoColon;"], "colon"),
        (&["--referer", "https://example.com/;auto"], "automatic"),
        (&["--cacert", "/tmp/ca.pem"], "--cacert"),
        (&["--write-out", "%{http_code}"], "--write-out"),
        (&["--"], "separator"),
        (&["--insecure"], "--insecure"),
    ] {
        let reason = reason(extra);
        assert!(reason.contains(expected), "{extra:?}: {reason}");
        assert!(!reason.contains("secret") && !reason.contains("example"));
    }
    for url in [
        "ftp://example.com/app.pkg",
        "https://user:pass@example.com/app.pkg",
        "https://example.com/app{1,2}.pkg",
        "https://example.com/app 1.pkg",
    ] {
        let mut command = Command::new("curl");
        command.args(["--location", "--url", url, "--output", "/tmp/out"]);
        command.env("SSL_CERT_FILE", "/tmp/bundle.pem");
        assert!(inspect(&command, false).is_err(), "{url}");
    }
    let mut command = generated(&[]);
    command.env("CURL_CA_BUNDLE", "/tmp/other.pem");
    assert!(inspect(&command, false)
        .unwrap_err()
        .contains("environment"));
}

#[test]
fn python_requests_use_their_generated_bundle() {
    let mut command = generated(&[
        "--cacert",
        "/tmp/a.pem",
        "--capath",
        "/tmp/certs",
        "--write-out",
        super::super::options::EFFECTIVE_URL_FORMAT,
    ]);
    command.env_remove("SSL_CERT_FILE");
    let request = inspect(&command, true).unwrap();
    assert!(request.write_out);
    assert_eq!(
        request.trust,
        super::super::options::Trust::Files {
            files: vec!["/tmp/a.pem".into()],
            directories: vec!["/tmp/certs".into()],
        }
    );
}

#[test]
fn preferences_select_curl() {
    let check = |value: Value| {
        let env = Dictionary::from_iter([(PREFERENCE.to_string(), value)]);
        policy(&env).map(|p| p.native)
    };
    for value in [
        Value::Boolean(false),
        Value::String("false".into()),
        Value::String("NO".into()),
        Value::Integer(0.into()),
    ] {
        assert_eq!(
            check(value).unwrap(),
            Err("UseRussetDownloader is false".to_string())
        );
    }
    assert!(check(Value::String("sometimes".into())).is_err());
    let env = Dictionary::from_iter([
        (PREFERENCE.to_string(), Value::Boolean(true)),
        (
            "CURL_PATH".to_string(),
            Value::String("/usr/bin/curl".into()),
        ),
    ]);
    assert_eq!(policy(&env).unwrap().native, Err("CURL_PATH is set".into()));
}
