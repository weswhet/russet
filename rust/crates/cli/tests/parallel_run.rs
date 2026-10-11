//! `run --jobs`: recipes overlap, reports keep list order, and a recipe
//! listed twice never overlaps itself. A local server stands in for
//! downloads and records which requests were in flight together.
use plist::{Dictionary, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Default)]
struct Requests {
    in_flight: HashMap<String, usize>,
    /// The most requests in flight at once, for each path and for all.
    most: HashMap<String, usize>,
    most_overall: usize,
    count: HashMap<String, usize>,
}

/// Serves `/NAME` after a delay, and 404 for `/missing`.
fn server() -> (String, Arc<Mutex<Requests>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Requests::default()));
    let shared = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let requests = shared.clone();
            thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut bytes = vec![];
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    bytes.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&bytes);
                let path = text.split(' ').nth(1).unwrap_or("").to_owned();
                {
                    let mut requests = requests.lock().unwrap();
                    *requests.count.entry(path.clone()).or_default() += 1;
                    let now = {
                        let count = requests.in_flight.entry(path.clone()).or_default();
                        *count += 1;
                        *count
                    };
                    let most = requests.most.entry(path.clone()).or_default();
                    *most = (*most).max(now);
                    let overall = requests.in_flight.values().sum();
                    requests.most_overall = requests.most_overall.max(overall);
                }
                thread::sleep(Duration::from_millis(if path == "/Slow" {
                    1500
                } else {
                    400
                }));
                let (status, body) = if path == "/missing" {
                    ("404 Not Found", String::new())
                } else {
                    ("200 OK", format!("contents of {path}"))
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                *requests.lock().unwrap().in_flight.get_mut(&path).unwrap() -= 1;
            });
        }
    });
    (url, requests)
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    prefs: PathBuf,
}

/// Download recipes named `NAME.download`, each fetching `/NAME`, and one
/// named `Missing.download` that fails.
fn fixture(names: &[&str], extra: &[(&str, Value)]) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    // Recipes see the resolved temporary folder, such as /private/var on
    // macOS. Windows would add a verbatim prefix.
    #[cfg(unix)]
    let root = temp.path().canonicalize().unwrap();
    #[cfg(not(unix))]
    let root = temp.path().to_owned();
    let prefs = root.join("prefs.plist");
    let mut values = Dictionary::from_iter([
        (
            "CACHE_DIR",
            Value::from(root.join("cache").to_string_lossy().into_owned()),
        ),
        (
            "RECIPE_MAP_PATH",
            Value::from(root.join("map.json").to_string_lossy().into_owned()),
        ),
        (
            "RECIPE_SEARCH_DIRS",
            Value::Array(vec![root
                .join("recipes")
                .to_string_lossy()
                .into_owned()
                .into()]),
        ),
        ("RECIPE_OVERRIDE_DIRS", Value::Array(vec![])),
    ]);
    values.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    Value::Dictionary(values).to_file_xml(&prefs).unwrap();
    fs::create_dir(root.join("recipes")).unwrap();
    for name in names {
        let path = if *name == "Missing" { "missing" } else { name };
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::from(format!("org.test.{name}"))),
            ("Input", Value::Dictionary(Dictionary::new())),
            (
                "Process",
                Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
                    ("Processor", Value::from("URLDownloader")),
                    (
                        "Arguments",
                        Value::Dictionary(Dictionary::from_iter([
                            ("url", Value::from(format!("%BASE%/{path}"))),
                            ("filename", Value::from(format!("{name}.txt"))),
                        ])),
                    ),
                ]))]),
            ),
        ]))
        .to_file_xml(root.join("recipes").join(format!("{name}.download.recipe")))
        .unwrap();
    }
    let fixture = Fixture {
        _temp: temp,
        root,
        prefs,
    };
    let map = fixture.command("generate-recipe-map").output().unwrap();
    assert!(map.status.success(), "{}", text(&map.stderr));
    fixture
}

impl Fixture {
    fn command(&self, verb: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_russet"));
        command
            .env_clear()
            .env("HOME", &self.root)
            .env("USERPROFILE", &self.root)
            .env("TMPDIR", &self.root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &self.prefs)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .arg(verb)
            .args(["--prefs", self.prefs.to_str().unwrap()]);
        command
    }

    fn run(&self, base: &str, args: &[&str]) -> Output {
        self.command("run")
            .env("AUTOPKG_BASE", base)
            .args(args)
            .output()
            .unwrap()
    }

    fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// The download path in each summary row, in order.
fn summary_rows(report: &Path) -> Vec<String> {
    let report = Value::from_file(report).unwrap();
    report.as_dictionary().unwrap()["summary_results"]
        .as_dictionary()
        .unwrap()["url_downloader_summary_result"]
        .as_dictionary()
        .unwrap()["data_rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_dictionary().unwrap()["download_path"]
                .as_string()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn parallel_runs_report_in_list_order_and_prefix_output() {
    let (base, requests) = server();
    let names = ["Slow", "Bravo", "Missing", "Delta", "Echo"];
    let fixture = fixture(&names, &[]);
    let recipes: Vec<_> = names.iter().map(|n| format!("{n}.download")).collect();
    let report = fixture.root.join("report.plist");
    let mut args = vec!["--jobs", "3", "--report-plist", report.to_str().unwrap()];
    args.extend(recipes.iter().map(String::as_str));
    let output = fixture.run(&base, &args);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert_eq!(output.status.code(), Some(70), "{stdout}\n{stderr}");
    assert!(
        requests.lock().unwrap().most_overall >= 2,
        "no recipes ran at the same time"
    );

    // Live output: every line a recipe writes starts with its name.
    let live = stdout.split("\n\n").next().unwrap();
    // Setup, before any recipe starts, isn't prefixed.
    for line in live
        .lines()
        .filter(|line| !line.starts_with("Recipe map path overridden"))
    {
        assert!(
            recipes.iter().any(|r| line.starts_with(&format!("[{r}] "))),
            "unprefixed line {line:?} in\n{stdout}"
        );
    }
    for recipe in &recipes {
        let line = format!("[{recipe}] Processing {recipe}...");
        assert!(live.lines().any(|l| l == line), "{live}");
    }
    assert!(stderr.contains("[Missing.download] Failed.\n"), "{stderr}");

    // Reports follow the list, even though Slow finished last.
    let downloads = fixture.cache();
    let expected: Vec<_> = ["Slow", "Bravo", "Delta", "Echo"]
        .iter()
        .map(|n| {
            downloads
                .join(format!("org.test.{n}"))
                .join("downloads")
                .join(format!("{n}.txt"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(summary_rows(&report), expected);
    let failures = Value::from_file(&report).unwrap().as_dictionary().unwrap()["failures"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].as_dictionary().unwrap()["recipe"].as_string(),
        Some("Missing.download")
    );
    let summary = stdout.split_once("\n\n").unwrap().1;
    let positions: Vec<_> = expected
        .iter()
        .map(|path| summary.find(path.as_str()).unwrap())
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{summary}");

    let results = Value::from_file(fixture.cache().join("autopkg_results.plist")).unwrap();
    let results = results.as_array().unwrap();
    assert_eq!(results.len(), names.len());
    for (receipt, name) in results.iter().zip(names) {
        let input = receipt.as_array().unwrap()[0].as_dictionary().unwrap()["Recipe input"]
            .as_dictionary()
            .unwrap();
        assert_eq!(
            input["RECIPE_PATH"].as_string(),
            fixture
                .root
                .join("recipes")
                .join(format!("{name}.download.recipe"))
                .to_str()
        );
    }
}

#[test]
fn a_recipe_listed_twice_never_overlaps_itself() {
    let (base, requests) = server();
    let fixture = fixture(&["Twice", "Other"], &[("RussetJobs", Value::from(3))]);
    let output = fixture.run(
        &base,
        &["Twice.download", "Other.download", "Twice.download"],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.count["/Twice"], 2);
    assert_eq!(
        requests.most["/Twice"], 1,
        "the same recipe ran twice at once"
    );
    assert!(requests.most_overall >= 2, "the preference didn't apply");
}

#[test]
fn one_job_matches_the_sequential_output() {
    let (base, _requests) = server();
    let names = ["Alpha", "Missing", "Bravo"];
    let fixture = fixture(&names, &[]);
    let recipes: Vec<_> = names.iter().map(|n| format!("{n}.download")).collect();
    let recipes: Vec<_> = recipes.iter().map(String::as_str).collect();
    let default = fixture.run(&base, &recipes);
    fs::remove_dir_all(fixture.cache()).unwrap();
    let mut args = vec!["--jobs", "1"];
    args.extend(&recipes);
    let one = fixture.run(&base, &args);
    assert_eq!(default.status.code(), Some(70));
    assert_eq!(one.status.code(), Some(70));
    assert_eq!(text(&default.stdout), text(&one.stdout));
    assert_eq!(text(&default.stderr), text(&one.stderr));
    assert!(text(&default.stdout).contains("\nProcessing Alpha.download...\n"));
    assert!(!text(&default.stdout).contains("[Alpha.download]"));
}

fn fixture_with_jobs(value: &str) -> Fixture {
    fixture(&["Alpha"], &[("RussetJobs", Value::from(value))])
}

#[test]
fn invalid_job_counts_are_usage_errors() {
    let fixture = fixture(&["Alpha"], &[]);
    for value in ["-1", "two", ""] {
        let output = fixture.run("http://127.0.0.1:9", &["--jobs", value, "Alpha.download"]);
        assert_eq!(output.status.code(), Some(2), "{value}");
        assert!(text(&output.stderr).contains("--jobs must be a whole number"));
    }
    let invalid = fixture_with_jobs("many");
    let output = invalid.run("http://127.0.0.1:9", &["Alpha.download"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("RussetJobs must be a whole number of 0 or more"));
}
