use plist::{Dictionary, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_russet"))
}

fn isolated_preferences(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("preferences.plist");
    Value::Dictionary(Dictionary::from_iter([
        (
            "CACHE_DIR",
            Value::from(root.join("cache").to_string_lossy().into_owned()),
        ),
        ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
        (
            "RECIPE_MAP_PATH",
            Value::from(root.join("map.json").to_string_lossy().into_owned()),
        ),
    ]))
    .to_file_xml(&path)
    .unwrap();
    path
}

#[test]
fn inventory_preserves_frozen_contract_and_adds_promoted_processors() {
    let result = command().arg("list-processors").output().unwrap();
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    let names: Vec<_> = output.lines().collect();
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../compatibility/reference.json")).unwrap();
    assert_eq!(contract["processors"].as_object().unwrap().len(), 46);
    assert!(contract["processors"]
        .as_object()
        .unwrap()
        .keys()
        .all(|name| names.contains(&name.as_str())));
    let merged = autopkg_processors::contract();
    let expected: Vec<_> = merged["processors"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, expected);
    assert_eq!(names.len(), 59);
}

#[test]
fn standalone_stdout_is_only_plist() {
    let root = tempfile::tempdir().unwrap();
    let preferences = isolated_preferences(root.path());
    let input = Value::Dictionary(Dictionary::from_iter([
        ("verbose", Value::Integer(2.into())),
        ("input_string", Value::String("hello planet".into())),
        ("find", "planet".into()),
        ("replace", "world".into()),
    ]));
    let mut child = command()
        .env("AUTOPKG_RS_PREFERENCES_FILE", &preferences)
        .env("HOME", root.path())
        .args(["processor-run", "FindAndReplace"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut bytes = Vec::new();
    input.to_writer_xml(&mut bytes).unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("FindAndReplace: Replacing"));
    let result = Value::from_reader(std::io::Cursor::new(result.stdout)).unwrap();
    assert_eq!(
        result.as_dictionary().unwrap()["output_string"].as_string(),
        Some("hello world")
    );
}

#[test]
fn whole_batch_is_validated_before_file_creation() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("created.txt");
    let good = dir.path().join("good.yaml");
    let bad = dir.path().join("bad.yaml");
    let target_json = serde_json::to_string(&target.to_string_lossy()).unwrap();
    std::fs::write(&good, format!("Input: {{}}\nProcess:\n- Processor: FileCreator\n  Arguments:\n    file_path: {target_json}\n    file_content: hello\n")).unwrap();
    std::fs::write(
        &bad,
        "Input: {}\nProcess:\n- Processor: example/CustomPython\n",
    )
    .unwrap();
    let result = command()
        .arg("run")
        .arg("--prefs")
        .arg(isolated_preferences(dir.path()))
        .env("HOME", dir.path())
        .env(
            "AUTOPKG_RS_PREFERENCES_FILE",
            dir.path().join("preferences.plist"),
        )
        .arg(&good)
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!target.exists());
}

#[test]
fn recipe_creates_unicode_file_with_cli_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("résultat.txt");
    let recipe = dir.path().join("test.yaml");
    let path = serde_json::to_string(&target.to_string_lossy()).unwrap();
    std::fs::write(&recipe, format!("Identifier: test\nInput:\n  CONTENT: recipe\nProcess:\n- Processor: FileCreator\n  Arguments:\n    file_path: {path}\n    file_content: '%CONTENT%'\n")).unwrap();
    let result = command()
        .args(["run", "--prefs"])
        .arg(isolated_preferences(dir.path()))
        .env("HOME", dir.path())
        .env(
            "AUTOPKG_RS_PREFERENCES_FILE",
            dir.path().join("preferences.plist"),
        )
        .args(["-k", "CONTENT=cli"])
        .arg(&recipe)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "cli");
}

#[test]
fn reports_processor_failures_with_recipe_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let recipe = dir.path().join("failure.yaml");
    let report = dir.path().join("report.plist");
    let path = serde_json::to_string(&dir.path().join("missing/file").to_string_lossy()).unwrap();
    std::fs::write(&recipe, format!("Identifier: test.failure\nInput: {{}}\nProcess:\n- Processor: FileCreator\n  Arguments:\n    file_path: {path}\n    file_content: hello\n")).unwrap();
    let output = command()
        .args(["run", "--prefs"])
        .arg(isolated_preferences(dir.path()))
        .env("HOME", dir.path())
        .env(
            "AUTOPKG_RS_PREFERENCES_FILE",
            dir.path().join("preferences.plist"),
        )
        .arg("--report-plist")
        .arg(&report)
        .arg(&recipe)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(70));
    let report = Value::from_file(report).unwrap();
    let failures = report.as_dictionary().unwrap()["failures"]
        .as_array()
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].as_dictionary().unwrap()["recipe_id"].as_string(),
        Some("test.failure")
    );
}

#[test]
fn standalone_runs_without_python_on_path() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("native.txt");
    let input = Value::Dictionary(Dictionary::from_iter([
        (
            "file_path",
            Value::String(target.to_string_lossy().into_owned()),
        ),
        ("file_content", Value::String("native".into())),
    ]));
    let mut child = command()
        .args(["processor-run", "FileCreator"])
        .env("PATH", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut bytes = Vec::new();
    input.to_writer_xml(&mut bytes).unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "native");
}

#[test]
fn recipe_inputs_override_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("precedence.txt");
    let prefs = dir.path().join("preferences.plist");
    Value::Dictionary(Dictionary::from_iter([
        ("CONTENT", Value::String("preference".into())),
        (
            "CACHE_DIR",
            Value::from(dir.path().join("cache").to_string_lossy().into_owned()),
        ),
        ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
        (
            "RECIPE_MAP_PATH",
            Value::from(dir.path().join("map.json").to_string_lossy().into_owned()),
        ),
    ]))
    .to_file_xml(&prefs)
    .unwrap();
    let recipe = dir.path().join("test.yaml");
    let path = serde_json::to_string(&target.to_string_lossy()).unwrap();
    std::fs::write(&recipe,format!("Identifier: org.test.precedence\nInput:\n  CONTENT: recipe\nProcess:\n- Processor: FileCreator\n  Arguments:\n    file_path: {path}\n    file_content: '%CONTENT%'\n")).unwrap();
    let output = command()
        .args(["run", "--prefs"])
        .env("HOME", dir.path())
        .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
        .arg(prefs)
        .arg(recipe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "recipe");
}
