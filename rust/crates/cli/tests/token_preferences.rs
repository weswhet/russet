use plist::{Dictionary, Value};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn server() -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "No GitHub request arrived");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = vec![];
        let mut byte = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            bytes.push(byte[0]);
        }
        let body = r#"{"name":"1.0","tag_name":"v1.0","prerelease":false,"body":null,"assets":[{"name":"App.zip","browser_download_url":"https://example.invalid/App.zip","url":"https://api.example.invalid/asset","created_at":"2026-01-01T00:00:00Z"}]}"#;
        let body = if String::from_utf8_lossy(&bytes).contains("/releases/latest ") {
            body.to_owned()
        } else {
            format!("[{body}]")
        };
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        String::from_utf8(bytes).unwrap()
    });
    (url, handle)
}
fn command(root: &Path, prefs: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"));
    command
        .current_dir(root)
        .env("HOME", root)
        .env("USERPROFILE", root)
        .env("AUTOPKG_RS_PREFERENCES_FILE", prefs)
        .env("TMPDIR", root)
        .env_remove("AUTOPKG_GITHUB_TOKEN");
    command
}
fn preferences(root: &Path, token: Option<&str>) -> std::path::PathBuf {
    let path = root.join("prefs.plist");
    let mut values = Dictionary::from_iter([
        (
            "CACHE_DIR",
            Value::from(root.join("cache").to_string_lossy().into_owned()),
        ),
        (
            "RECIPE_MAP_PATH",
            Value::from(root.join("map.json").to_string_lossy().into_owned()),
        ),
        ("RECIPE_SEARCH_DIRS", Value::Array(vec![])),
        ("RECIPE_OVERRIDE_DIRS", Value::Array(vec![])),
        ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
    ]);
    if let Some(token) = token {
        values.insert("GITHUB_TOKEN".into(), Value::from(token));
    }
    Value::Dictionary(values).to_file_xml(&path).unwrap();
    path
}
fn input(root: &Path, url: String) -> Dictionary {
    let token_path = root.join("token");
    std::fs::write(&token_path, "file-token").unwrap();
    Dictionary::from_iter([
        ("github_repo", Value::from("owner/project")),
        ("GITHUB_URL", Value::from(url)),
        (
            "GITHUB_TOKEN_PATH",
            Value::from(token_path.to_string_lossy().into_owned()),
        ),
        ("GITHUB_TOKEN", Value::from("recipe-token")),
        (
            "curl_opts",
            Value::Array(vec![Value::from("--noproxy"), Value::from("*")]),
        ),
    ])
}
#[test]
fn recipe_and_cli_tokens_never_override_authentication_preferences() {
    for preference in [None, Some("preference-token")] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let prefs = preferences(root, preference);
        let (url, request) = server();
        let recipe = root.join("Token.recipe");
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::from("org.test.token")),
            ("Input", Value::Dictionary(input(root, url))),
            (
                "Process",
                Value::Array(vec![
                    Value::Dictionary(Dictionary::from_iter([(
                        "Processor",
                        Value::from("GitHubReleasesInfoProvider"),
                    )])),
                    Value::Dictionary(Dictionary::from_iter([
                        ("Processor", Value::from("FileCreator")),
                        (
                            "Arguments",
                            Value::Dictionary(Dictionary::from_iter([
                                (
                                    "file_path",
                                    Value::from(
                                        root.join("preserved.txt").to_string_lossy().into_owned(),
                                    ),
                                ),
                                ("file_content", Value::from("%GITHUB_TOKEN%")),
                            ])),
                        ),
                    ])),
                ]),
            ),
        ]))
        .to_file_xml(&recipe)
        .unwrap();
        let output = command(root, &prefs)
            .args(["run", "--prefs"])
            .arg(&prefs)
            .args(["-k", "GITHUB_TOKEN=cli-token"])
            .arg(&recipe)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let request = request.join().unwrap();
        assert!(request.contains(&format!(
            "Authorization: token {}",
            preference.unwrap_or("file-token")
        )));
        assert_eq!(
            std::fs::read_to_string(root.join("preserved.txt")).unwrap(),
            "cli-token"
        );
        assert!(!request.contains("recipe-token"));
        assert!(!request.contains("cli-token"));
        let receipts = Value::from_file(root.join("cache/autopkg_results.plist")).unwrap();
        let initial = &receipts.as_array().unwrap()[0].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap()["Recipe input"];
        assert!(!initial
            .as_dictionary()
            .unwrap()
            .contains_key("GITHUB_TOKEN"));
    }
}
#[test]
fn standalone_preferences_are_separate_and_output_environment_is_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let prefs = preferences(root, Some("preference-token"));
    let (url, request) = server();
    let input = Value::Dictionary(input(root, url));
    let mut bytes = vec![];
    input.to_writer_xml(&mut bytes).unwrap();
    let mut child = command(root, &prefs)
        .args(["processor-run", "GitHubReleasesInfoProvider"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(request
        .join()
        .unwrap()
        .contains("Authorization: token preference-token"));
    let result = Value::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
    assert_eq!(
        result.as_dictionary().unwrap()["GITHUB_TOKEN"].as_string(),
        Some("recipe-token")
    );
}
