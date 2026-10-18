//! Recipes that fail before they run: no check phase, or failed trust.
//! AutoPkg reports each as a failed recipe and goes on with the others.
use plist::{Dictionary, Value};
use std::{path::Path, process::Output};

struct Home {
    root: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        for name in ["recipes", "overrides", "more"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        let path = |name: &str| Value::from(root.path().join(name).to_string_lossy().into_owned());
        Value::Dictionary(Dictionary::from_iter([
            ("CACHE_DIR", path("cache")),
            ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
            ("RECIPE_MAP_PATH", path("map.json")),
            ("RECIPE_SEARCH_DIRS", Value::Array(vec![path("recipes")])),
            (
                "RECIPE_OVERRIDE_DIRS",
                Value::Array(vec![path("overrides")]),
            ),
            ("RECIPE_REPO_DIR", path("repos")),
        ]))
        .to_file_xml(root.path().join("prefs.plist"))
        .unwrap();
        let home = Self { root };
        home.recipe("recipes/Parent.recipe.yaml", "org.test.parent", "created");
        home
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.root.path().join(name)
    }

    fn recipe(&self, name: &str, identifier: &str, content: &str) {
        let target = serde_json::to_string(&self.path("created.txt").to_string_lossy()).unwrap();
        std::fs::write(
            self.path(name),
            format!("Identifier: {identifier}\nInput: {{}}\nProcess:\n- Processor: FileCreator\n  Arguments:\n    file_path: {target}\n    file_content: {content}\n"),
        )
        .unwrap();
    }

    fn russet(&self, args: &[&str]) -> Output {
        std::process::Command::new(env!("CARGO_BIN_EXE_russet"))
            .args(&args[..1])
            .arg("--prefs")
            .arg(self.path("prefs.plist"))
            .args(&args[1..])
            .current_dir(self.root.path())
            .env("HOME", self.path("home"))
            .env("RUSSET_PREFERENCES_FILE", self.path("prefs.plist"))
            .output()
            .unwrap()
    }

    fn results(&self) -> Value {
        Value::from_file(self.path("cache/autopkg_results.plist")).unwrap()
    }
}

/// Output with Windows line endings normalized.
fn text(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).unwrap().replace("\r\n", "\n")
}

#[test]
fn check_without_a_check_phase_skips_the_recipe_and_fails_the_run() {
    let home = Home::new();
    let output = home.russet(&["run", "--check", "Parent"]);
    assert_eq!(output.status.code(), Some(70));
    assert_eq!(
        text(&output.stderr),
        "Recipe at Parent is missing EndOfCheckPhase Processor, not possible to perform check.\n"
    );
    assert!(!text(&output.stdout).contains("Processing"));
    assert!(text(&output.stdout).ends_with("\nNothing downloaded, packaged or imported.\n"));
    assert_eq!(home.results(), Value::Array(vec![]));
    assert!(!home.path("created.txt").exists());
}

#[test]
fn failed_parent_trust_is_a_failed_recipe() {
    let home = Home::new();
    assert!(home.russet(&["make-override", "Parent"]).status.success());
    home.recipe("recipes/Parent.recipe.yaml", "org.test.parent", "changed");
    let parent = home.path("recipes/Parent.recipe.yaml");
    let expected = format!(
        "Parent recipe org.test.parent contents differ from expected.\n    Path: {}",
        parent.canonicalize().unwrap().display()
    );

    let output = home.russet(&["run", "Parent"]);
    assert_eq!(output.status.code(), Some(70));
    assert_eq!(text(&output.stderr), "Failed local trust verification.\n");
    let stdout = text(&output.stdout);
    assert!(stdout.contains("Processing Parent...\n"));
    let summary = expected.replace('\n', "\n        ");
    assert!(
        stdout.contains(&format!(
            "The following recipes failed:\n    Parent\n        {summary}\n"
        )),
        "{stdout}"
    );
    assert_eq!(
        home.results(),
        Value::Array(vec![Value::Array(vec![Value::Dictionary(
            Dictionary::from_iter([("RecipeError", Value::String(expected.clone()))])
        )])])
    );
    assert!(!home.path("created.txt").exists());

    // The same detail, from a plain-text recipe list.
    std::fs::write(home.path("list.txt"), "# Recipes\nParent\n\n").unwrap();
    let output = home.russet(&["verify-trust-info", "-v", "-l", "list.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        format!("Parent: FAILED\n    {}\n", expected.replace('\n', "\n    "))
    );

    let output = home.russet(&["run", "--ignore-parent-trust-verification-errors", "Parent"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        std::fs::read_to_string(home.path("created.txt")).unwrap(),
        "changed"
    );
}

#[test]
fn recipes_see_the_folders_given_on_the_command_line() {
    let home = Home::new();
    home.recipe("more/More.recipe.yaml", "org.test.more", "more");
    let more = home.path("more");
    let output = home.russet(&["run", "-d", more.to_str().unwrap(), "More"]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let results = home.results();
    let input = results.as_array().unwrap()[0].as_array().unwrap()[0]
        .as_dictionary()
        .unwrap()["Recipe input"]
        .as_dictionary()
        .unwrap()
        .clone();
    let strings = |paths: &[&Path]| {
        Value::Array(
            paths
                .iter()
                .map(|path| Value::from(path.to_string_lossy().into_owned()))
                .collect(),
        )
    };
    assert_eq!(input["RECIPE_SEARCH_DIRS"], strings(&[&more]));
    assert_eq!(
        input["RECIPE_OVERRIDE_DIRS"],
        strings(&[&home.path("overrides")])
    );
}
