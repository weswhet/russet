use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Sandbox {
    temp: tempfile::TempDir,
    autopkg: PathBuf,
    russet: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let autopkg = temp.path().join("autopkg.plist");
        Value::Dictionary(Dictionary::from_iter([
            (
                "RECIPE_REPO_DIR",
                Value::from("~/Library/AutoPkg/RecipeRepos"),
            ),
            (
                "RECIPE_SEARCH_DIRS",
                Value::Array(vec![".".into(), "~/Library/AutoPkg/Recipes".into()]),
            ),
            ("GITHUB_TOKEN", Value::from("autopkg-token")),
        ]))
        .to_file_xml(&autopkg)
        .unwrap();
        let russet = temp.path().join("config/russet.plist");
        Self {
            temp,
            autopkg,
            russet,
        }
    }
    fn migrate(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_russet"))
            .env("HOME", self.temp.path())
            .env("USERPROFILE", self.temp.path())
            .env("RUSSET_AUTOPKG_PREFERENCES_FILE", &self.autopkg)
            .env("RUSSET_PREFERENCES_FILE", &self.russet)
            .arg("migrate")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn values(path: &Path) -> Dictionary {
    Value::from_file(path).unwrap().into_dictionary().unwrap()
}

#[test]
fn migrate_copies_autopkg_preferences_without_changing_them() {
    let sandbox = Sandbox::new();
    let before = fs::read(&sandbox.autopkg).unwrap();

    let dry_run = stdout(&sandbox.migrate(&["--dry-run"]));
    assert!(dry_run.contains("GITHUB_TOKEN"), "{dry_run}");
    assert!(dry_run.contains("3 copied"), "{dry_run}");
    assert!(!sandbox.russet.exists());

    let run = stdout(&sandbox.migrate(&[]));
    assert!(run.contains("3 copied"), "{run}");
    assert_eq!(values(&sandbox.russet), values(&sandbox.autopkg));

    let again = stdout(&sandbox.migrate(&[]));
    assert!(
        again.contains("0 copied, 0 replaced, 3 unchanged"),
        "{again}"
    );
    assert_eq!(fs::read(&sandbox.autopkg).unwrap(), before);
}

#[test]
fn migrate_keeps_russet_values_unless_forced() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.russet.parent().unwrap()).unwrap();
    Value::Dictionary(Dictionary::from_iter([
        ("GITHUB_TOKEN", Value::from("russet-token")),
        ("RUSSET_ONLY", Value::Boolean(true)),
    ]))
    .to_file_xml(&sandbox.russet)
    .unwrap();

    let run = stdout(&sandbox.migrate(&[]));
    assert!(run.contains("1 conflicting"), "{run}");
    let saved = values(&sandbox.russet);
    assert_eq!(saved["GITHUB_TOKEN"].as_string(), Some("russet-token"));
    assert_eq!(saved["RUSSET_ONLY"].as_boolean(), Some(true));
    assert!(saved.contains_key("RECIPE_REPO_DIR"));

    stdout(&sandbox.migrate(&["--force"]));
    let saved = values(&sandbox.russet);
    assert_eq!(saved["GITHUB_TOKEN"].as_string(), Some("autopkg-token"));
    assert_eq!(saved["RUSSET_ONLY"].as_boolean(), Some(true));
}

#[test]
fn migrate_keeps_every_value_type_and_autopkgs_default_folders() {
    let sandbox = Sandbox::new();
    fs::create_dir_all(sandbox.temp.path().join("Library/AutoPkg/RecipeOverrides")).unwrap();
    let date =
        plist::Date::from(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000));
    let custom = Dictionary::from_iter([
        ("STRING", Value::from("text")),
        ("INTEGER", Value::from(3)),
        ("REAL", Value::Real(1.5)),
        ("BOOLEAN", Value::Boolean(false)),
        ("DATE", Value::Date(date)),
        ("DATA", Value::Data(vec![0, 1, 2, 255])),
        (
            "NESTED",
            Value::Dictionary(Dictionary::from_iter([(
                "list",
                Value::Array(vec![Value::from("a"), Value::Dictionary(Dictionary::new())]),
            )])),
        ),
    ]);
    let mut autopkg = values(&sandbox.autopkg);
    autopkg.extend(custom.clone());
    Value::Dictionary(autopkg.clone())
        .to_file_xml(&sandbox.autopkg)
        .unwrap();

    let run = stdout(&sandbox.migrate(&[]));
    assert!(
        run.contains("RECIPE_OVERRIDE_DIRS  copied (AutoPkg's default)"),
        "{run}"
    );
    let saved = values(&sandbox.russet);
    for (key, value) in custom.iter().chain(autopkg.iter()) {
        assert_eq!(saved.get(key), Some(value), "{key}");
    }
    assert_eq!(
        saved["RECIPE_OVERRIDE_DIRS"],
        Value::Array(vec!["~/Library/AutoPkg/RecipeOverrides".into()])
    );
    let again = stdout(&sandbox.migrate(&[]));
    assert!(
        again.contains("0 copied, 0 replaced, 11 unchanged"),
        "{again}"
    );
}
