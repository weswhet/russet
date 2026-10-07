use plist::{Dictionary, Value};
use std::{fs, path::Path, process::Command};
fn save(path: &Path, values: Dictionary) {
    Value::Dictionary(values).to_file_xml(path).unwrap();
}
fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().map(|s| Value::String((*s).into())).collect())
}
#[test]
fn recipe_list_precedence_receipts_and_install_alias() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let cache = root.join("cache");
    let prefs = root.join("prefs.plist");
    save(
        &prefs,
        Dictionary::from_iter([
            ("CACHE_DIR", cache.to_string_lossy().to_string().into()),
            (
                "RECIPE_MAP_PATH",
                root.join("map.json").to_string_lossy().to_string().into(),
            ),
            ("RECIPE_SEARCH_DIRS", strings(&[root.to_str().unwrap()])),
            ("RECIPE_OVERRIDE_DIRS", strings(&[])),
        ]),
    );
    let recipe = root.join("Fixture.install.recipe");
    save(
        &recipe,
        Dictionary::from_iter([
            ("Identifier", Value::from("org.test.fixture")),
            (
                "Input",
                Value::Dictionary(Dictionary::from_iter([("NAME", Value::from("recipe"))])),
            ),
            (
                "Process",
                Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
                    ("Processor", Value::from("FileCreator")),
                    (
                        "Arguments",
                        Value::Dictionary(Dictionary::from_iter([
                            ("file_path", Value::from("%RECIPE_CACHE_DIR%/result")),
                            ("file_content", Value::from("%NAME%")),
                        ])),
                    ),
                ]))]),
            ),
        ]),
    );
    let list = root.join("list.plist");
    save(
        &list,
        Dictionary::from_iter([
            ("recipes", strings(&[recipe.to_str().unwrap()])),
            ("NAME", Value::from("list")),
        ]),
    );
    let report = root.join("report.plist");
    let invoke = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
            .env_clear()
            .env("HOME", root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("AUTOPKG_NAME", "environment")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    invoke(&[
        "run",
        "--prefs",
        prefs.to_str().unwrap(),
        "-l",
        list.to_str().unwrap(),
        "--report-plist",
        report.to_str().unwrap(),
        "-k",
        "NAME=command",
    ]);
    assert_eq!(
        fs::read_to_string(cache.join("org.test.fixture/result")).unwrap(),
        "command"
    );
    let results = Value::from_file(cache.join("autopkg_results.plist")).unwrap();
    assert_eq!(results.as_array().unwrap().len(), 1);
    let receipts: Vec<_> = fs::read_dir(cache.join("org.test.fixture/receipts"))
        .unwrap()
        .collect();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        Value::from_file(receipts[0].as_ref().unwrap().path()).unwrap(),
        results.as_array().unwrap()[0]
    );
    assert!(Value::from_file(report).unwrap().as_dictionary().is_some());
    invoke(&[
        "run",
        "--prefs",
        prefs.to_str().unwrap(),
        "-l",
        list.to_str().unwrap(),
    ]);
    assert_eq!(
        fs::read_to_string(cache.join("org.test.fixture/result")).unwrap(),
        "list"
    );
    invoke(&["install", "--prefs", prefs.to_str().unwrap(), "Fixture"]);
    assert_eq!(
        fs::read_to_string(cache.join("org.test.fixture/result")).unwrap(),
        "environment"
    );
    let text = root.join("list.txt");
    fs::write(&text, format!("# comment\n\n{}\n", recipe.display())).unwrap();
    invoke(&[
        "run",
        "--prefs",
        prefs.to_str().unwrap(),
        "-l",
        text.to_str().unwrap(),
    ]);
}

#[test]
fn explicit_trust_bypass_keeps_validation_and_option_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let cache = root.join("cache");
    let prefs = root.join("prefs.plist");
    let parent = root.join("Parent.recipe");
    let child = root.join("Override.recipe");
    let write = |path: &Path, value: serde_json::Value| {
        serde_json::from_value::<Value>(value)
            .unwrap()
            .to_file_xml(path)
            .unwrap();
    };
    write(
        &prefs,
        serde_json::json!({"CACHE_DIR":cache,"RECIPE_MAP_PATH":root.join("map.json"),"RECIPE_SEARCH_DIRS":[root],"RECIPE_OVERRIDE_DIRS":[root],"FAIL_RECIPES_WITHOUT_TRUST_INFO":true}),
    );
    write(
        &parent,
        serde_json::json!({"Identifier":"org.parent","Input":{},"Process":[]}),
    );
    write(
        &child,
        serde_json::json!({"Identifier":"org.child","ParentRecipe":"org.parent","Input":{},"Process":[],"ParentRecipeTrustInfo":{}}),
    );
    let invoke = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
            .env_clear()
            .env("HOME", root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
            .args(["run", &format!("--prefs={}", prefs.display())])
            .args(extra)
            .arg(&child)
            .output()
            .unwrap()
    };
    assert!(!invoke(&[]).status.success());
    assert!(!cache.exists());
    let output = invoke(&[
        "-vv",
        "--ignore-parent-trust-verification-errors",
        "--pkg=provided",
        "-kPKG=ignored",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results = Value::from_file(cache.join("autopkg_results.plist")).unwrap();
    let inputs = &results.as_array().unwrap()[0].as_array().unwrap()[0]
        .as_dictionary()
        .unwrap()["Recipe input"];
    let inputs = inputs.as_dictionary().unwrap();
    assert_eq!(inputs["verbose"].as_unsigned_integer(), Some(2));
    assert_eq!(inputs["PKG"].as_string(), Some("provided"));
    write(
        &parent,
        serde_json::json!({"Identifier":"org.parent","Input":{},"Process":[{"Processor":"CustomPython"}]}),
    );
    assert!(!invoke(&["--ignore-parent-trust-verification-errors"])
        .status
        .success());
    let help = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
        .args(["run", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8_lossy(&help.stdout).contains("--ignore-parent-trust-verification-errors")
    );
}

#[test]
fn standalone_omits_top_level_binary_null_and_serializes_nested_null() {
    use std::io::Write;
    use std::process::Stdio;
    // bplist root {nullable: null, nested: [null]}; object 3 is the shared null.
    let mut bytes = b"bplist00".to_vec();
    bytes.extend_from_slice(&[0xd2, 1, 2, 3, 4, 0x58]);
    bytes.extend_from_slice(b"nullable");
    bytes.push(0x56);
    bytes.extend_from_slice(b"nested");
    bytes.extend_from_slice(&[0, 0xa1, 3]);
    assert_eq!(bytes.len(), 32);
    bytes.extend_from_slice(&[8, 13, 22, 29, 30]);
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 1]);
    bytes.extend_from_slice(&5u64.to_be_bytes());
    bytes.extend_from_slice(&0u64.to_be_bytes());
    bytes.extend_from_slice(&32u64.to_be_bytes());
    let mut child = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
        .args(["processor-run", "VariableSetter"])
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
    let value = Value::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
    let environment = value.as_dictionary().unwrap();
    assert!(!environment.contains_key("nullable"));
    assert_eq!(
        environment["nested"].as_array().unwrap()[0].as_string(),
        Some("")
    );
}

#[test]
fn default_and_development_caches_resolve_in_isolated_home() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    let prefs = temp.path().join("prefs.plist");
    let recipe = temp.path().join("Default.recipe");
    save(
        &prefs,
        Dictionary::from_iter([
            ("CACHE_DIR", Value::from("")),
            ("RECIPE_SEARCH_DIRS", strings(&[])),
            ("RECIPE_OVERRIDE_DIRS", strings(&[])),
            ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
        ]),
    );
    save(
        &recipe,
        Dictionary::from_iter([
            ("Identifier", Value::from("org.default-cache")),
            ("Input", Value::Dictionary(Dictionary::new())),
            ("Process", Value::Array(vec![])),
        ]),
    );
    let invoke = |verb: &str, extra: &[&str], developer: Option<&Path>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"));
        command
            .env_clear()
            .env("HOME", &home)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
            .env("USERPROFILE", &home)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .arg(verb)
            .args(["--prefs", prefs.to_str().unwrap()])
            .args(extra)
            .current_dir(temp.path());
        if let Some(path) = developer {
            command.env("AUTOPKG_RS_CACHE_DIR", path);
        }
        command.output().unwrap()
    };
    let output = invoke("run", &[recipe.to_str().unwrap()], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let root = home.join("Library/AutoPkg/Cache");
    assert!(root.join("autopkg_results.plist").is_file());
    assert!(root.join("org.default-cache/receipts").is_dir());
    let output = invoke("clear-cache", &["--dry-run", "all"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join("autopkg_results.plist").is_file());
    let output = invoke("clear-cache", &["all"], None);
    assert!(output.status.success());
    assert!(root.exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    let developer = temp.path().join("separate");
    let output = invoke("run", &[recipe.to_str().unwrap()], Some(&developer));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(developer.join("autopkg_results.plist").is_file());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    // Explicit per-recipe roots do not redirect the preference-level run report.
    let output = invoke(
        "run",
        &[
            "-k",
            "CACHE_DIR=~/custom/../recipes",
            recipe.to_str().unwrap(),
        ],
        Some(&developer),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.join("recipes/org.default-cache/receipts").is_dir());
    assert!(developer.join("autopkg_results.plist").is_file());
}

/// Exercises parsing, typed argument injection, processor I/O and the persisted
/// receipt serializer together, with the expected values produced by c36e58f.
///
/// `Typed.recipe.bplist` is the binary plist recipe built from TYPED_YAML's
/// values. `expected.plist` is the pinned `plist_serializer` output for that
/// recipe's Input. When they were captured with Python 3.11.9, the pinned
/// `AutoPkgYAMLLoader` read TYPED_YAML's Input to the same values.
#[test]
fn typed_plist_and_yaml_values_survive_cli_receipts() {
    const TYPED_YAML: &str = r#"Identifier: org.test.typed.yaml
Input:
  TopNull: null
  Typed: &typed
    blob: !!binary AAEC/w==
    date: 2026-10-06T12:34:56
    enabled: true
    disabled: false
    integer: 42
    negative: -7
    zero: 0
    real: !!float 2.5
    version: 2.3
    nullable: null
    nested:
      array: [null, false, 0, {blob: !!binary /wA=, text: "Café"}]
Process:
  - Processor: MunkiPkginfoMerger
    Arguments:
      additional_pkginfo: *typed
"#;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("Typed.recipe"),
        include_bytes!("fixtures/typed-receipts/Typed.recipe.bplist"),
    )
    .unwrap();
    fs::write(root.join("Typed.recipe.yaml"), TYPED_YAML).unwrap();
    let expected = Value::from_reader(std::io::Cursor::new(
        include_bytes!("fixtures/typed-receipts/expected.plist").as_slice(),
    ))
    .unwrap()
    .into_dictionary()
    .unwrap();
    let typed = &expected["Typed"];
    // Check the oracle's tags too, so equality cannot hide coercion to strings.
    let fields = typed.as_dictionary().unwrap();
    assert_eq!(
        fields["blob"].as_data(),
        Some(b"\x00\x01\x02\xff".as_slice())
    );
    assert!(fields["date"].as_date().is_some());
    assert_eq!(fields["disabled"].as_boolean(), Some(false));
    assert_eq!(fields["zero"].as_signed_integer(), Some(0));
    assert_eq!(fields["real"].as_real(), Some(2.5));
    assert_eq!(fields["version"].as_string(), Some("2.3"));
    assert_eq!(fields["nullable"].as_string(), Some(""));
    let cache = root.join("cache");
    let prefs = root.join("prefs.plist");
    save(
        &prefs,
        Dictionary::from_iter([
            (
                "CACHE_DIR",
                Value::String(cache.to_string_lossy().into_owned()),
            ),
            ("RECIPE_SEARCH_DIRS", strings(&[])),
            ("RECIPE_OVERRIDE_DIRS", strings(&[])),
            ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
        ]),
    );
    for (filename, identifier) in [
        ("Typed.recipe", "org.test.typed.plist"),
        ("Typed.recipe.yaml", "org.test.typed.yaml"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
            .env_clear()
            .env("HOME", root)
            .env("USERPROFILE", root)
            .env("CFFIXED_USER_HOME", root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &prefs)
            .args(["run", "--prefs"])
            .arg(&prefs)
            .arg(root.join(filename))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{filename}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let results = Value::from_file(cache.join("autopkg_results.plist")).unwrap();
        let runs = results.as_array().unwrap();
        assert_eq!(runs.len(), 1);
        let receipts: Vec<_> = fs::read_dir(cache.join(identifier).join("receipts"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(receipts.len(), 1);
        let persisted = Value::from_file(&receipts[0]).unwrap();
        assert_eq!(persisted, runs[0]);
        let entries = persisted.as_array().unwrap();
        assert_eq!(entries.len(), 2);
        let input = entries[0].as_dictionary().unwrap()["Recipe input"]
            .as_dictionary()
            .unwrap();
        assert_eq!(&input["Typed"], typed, "{filename}: recipe input");
        assert_eq!(input["TopNull"], expected["TopNull"]);
        let processor = entries[1].as_dictionary().unwrap();
        assert_eq!(
            processor["Processor"].as_string(),
            Some("MunkiPkginfoMerger")
        );
        assert_eq!(
            &processor["Input"].as_dictionary().unwrap()["additional_pkginfo"],
            typed,
            "{filename}: typed argument injection"
        );
        assert_eq!(
            &processor["Output"].as_dictionary().unwrap()["pkginfo"],
            typed,
            "{filename}: processor output persisted without coercion"
        );
    }
}
