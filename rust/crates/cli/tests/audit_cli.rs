use plist::Value;
use serde_json::json;
use std::{fs, process::Command};
#[test]
fn audit_inheritance_filters_and_machine_reports_do_not_execute_custom_code() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let parent = root.join("Parent.recipe.yaml");
    let child = root.join("Child.recipe.yaml");
    let prefs = root.join("prefs.plist");
    fs::write(&parent,"Identifier: org.parent\nInput:\n  URL: http://example.invalid/file\n  API_KEY: secret-never-echo\nProcess:\n  - Processor: URLDownloader\n  - Processor: NeverExecuteCustomCode\n").unwrap();
    fs::write(&child,"Identifier: org.child\nParentRecipe: org.parent\nParentRecipeTrustInfo: {}\nInput:\n  NAME: Child\nProcess: []\n").unwrap();
    let pref:Value=serde_json::from_value(json!({"RECIPE_SEARCH_DIRS":[root],"RECIPE_OVERRIDE_DIRS":[root],"DISABLE_RECIPE_MAP":true})).unwrap();
    pref.to_file_xml(&prefs).unwrap();
    let invoke = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
            .env("HOME", root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
            .args(["audit", "--prefs", prefs.to_str().unwrap()])
            .args(args)
            .arg(&child)
            .output()
            .unwrap()
    };
    let output = invoke(&["--json", "--fail-on", "error"]);
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("secret-never-echo"));
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report[0]["findings"].as_array().unwrap().len(), 4);
    assert_eq!(report[0]["findings"][0]["check"], "sensitive_input");
    let output = invoke(&[
        "--plist",
        "--only-check",
        "non_core_processor",
        "--fail-on",
        "warning",
    ]);
    assert!(output.status.success());
    let report = Value::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
    let result = &report.as_dictionary().unwrap()[child.to_str().unwrap()];
    assert_eq!(result.as_dictionary().unwrap().len(), 1);
    assert_eq!(
        result.as_dictionary().unwrap()["non_core_processors"]
            .as_array()
            .unwrap()[0]
            .as_string(),
        Some("NeverExecuteCustomCode")
    );
}
