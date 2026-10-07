use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Sandbox {
    _temp: tempfile::TempDir,
    home: PathBuf,
    prefs: PathBuf,
}
impl Sandbox {
    /// A home folder with one configured recipe repository that keeps its
    /// recipe in a vendor subfolder, as cloned repositories do.
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let autopkg = home.join("Library/AutoPkg");
        let repo = autopkg.join("RecipeRepos/com.example.recipes");
        fs::create_dir_all(repo.join("Vendor")).unwrap();
        fs::create_dir_all(autopkg.join("RecipeOverrides")).unwrap();
        fs::write(
            repo.join("Vendor/App.download.recipe.yaml"),
            "Identifier: com.example.download.App\nInput:\n  NAME: App\nProcess: []\n",
        )
        .unwrap();
        let prefs = temp.path().join("prefs.plist");
        let repos = Dictionary::from_iter([(
            repo.to_string_lossy().into_owned(),
            Value::Dictionary(Dictionary::from_iter([(
                "URL",
                Value::from("https://example.invalid/recipes"),
            )])),
        )]);
        Value::Dictionary(Dictionary::from_iter([
            (
                "CACHE_DIR",
                Value::from(temp.path().join("cache").to_string_lossy().into_owned()),
            ),
            (
                "RECIPE_MAP_PATH",
                Value::from(temp.path().join("map.json").to_string_lossy().into_owned()),
            ),
            (
                "RECIPE_SEARCH_DIRS",
                Value::Array(vec![".".into(), repo.to_string_lossy().into_owned().into()]),
            ),
            ("RECIPE_REPOS", Value::Dictionary(repos)),
            ("FAIL_RECIPES_WITHOUT_TRUST_INFO", Value::Boolean(false)),
        ]))
        .to_file_xml(&prefs)
        .unwrap();
        Self {
            _temp: temp,
            home,
            prefs,
        }
    }
    fn repo(&self) -> PathBuf {
        self.home
            .join("Library/AutoPkg/RecipeRepos/com.example.recipes")
    }
    fn autopkg(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_autopkg-rs"))
            .env_clear()
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("AUTOPKG_RS_PREFERENCES_FILE", &self.prefs)
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap()
    }
}
fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn run_finds_repository_recipes_by_short_name() {
    let sandbox = Sandbox::new();
    let output = sandbox.autopkg(&sandbox.home, &["run", "App.download"]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Processing App.download..."));
}

#[test]
fn overrides_keep_trust_when_run_from_a_folder_that_contains_them() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.autopkg(&sandbox.home, &["make-override", "App.download"]));
    assert!(sandbox
        .home
        .join("Library/AutoPkg/RecipeOverrides/App.download.recipe")
        .is_file());
    // The home folder is the search folder `.`, and it contains the override
    // folder; that must not make the override look like repository content.
    let mut folders = vec![sandbox.home.clone()];
    if cfg!(unix) {
        folders.push(PathBuf::from("/"));
    }
    for cwd in &folders {
        let output = sandbox.autopkg(cwd, &["verify-trust-info", "App.download"]);
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line == "App.download: OK"));
        let output = sandbox.autopkg(cwd, &["run", "App.download"]);
        assert_success(&output);
        assert!(!String::from_utf8_lossy(&output.stderr).contains("missing trust info"));
    }
    assert_success(&sandbox.autopkg(&sandbox.home, &["update-trust-info", "App.download"]));
}

#[test]
fn overrides_inside_recipe_repositories_stay_untrusted() {
    let sandbox = Sandbox::new();
    let inside = sandbox.repo().join("Overrides");
    fs::create_dir_all(&inside).unwrap();
    let inside = inside.to_string_lossy().into_owned();
    assert_success(&sandbox.autopkg(
        &sandbox.home,
        &["make-override", "--override-dir", &inside, "App.download"],
    ));
    for args in [
        vec!["run", "--override-dir", &inside, "App.download"],
        vec![
            "verify-trust-info",
            "-v",
            "--override-dir",
            &inside,
            "App.download",
        ],
        vec![
            "update-trust-info",
            "--override-dir",
            &inside,
            "App.download",
        ],
    ] {
        let output = sandbox.autopkg(&sandbox.home, &args);
        assert!(
            !output.status.success(),
            "{args:?} trusted a repository override"
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains(
                "Trust records must come from a local override outside recipe repositories"
            ),
            "{args:?}: {text}"
        );
    }
}
