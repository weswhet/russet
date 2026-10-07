//! Verification of local overrides and explicitly promoted processor sources.
//! Recipe and package-script hashes retain the reference's persistent format.
use crate::{load_inner, locate, read_recipe, Recipe};
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default)]
pub struct TrustPolicy {
    pub override_dirs: Vec<PathBuf>,
    pub repository_dirs: Vec<PathBuf>,
}

fn is_under(path: &Path, dirs: &[PathBuf]) -> bool {
    dirs.iter()
        .filter_map(|p| p.canonicalize().ok())
        .any(|root| path.starts_with(root))
}

fn collect(
    path: &Path,
    dirs: &[PathBuf],
    active: &mut HashSet<PathBuf>,
    graph: &mut BTreeMap<String, (PathBuf, String)>,
) -> Result<(), String> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !active.insert(path.clone()) {
        return Err("Recipe inheritance cycle in trust graph".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let data = read_recipe(&path)?;
    if data.contains_key("ParentRecipeTrustInfo") {
        return Err("Parent recipes cannot supply their own trust records".into());
    }
    let identifier = data
        .get("Identifier")
        .and_then(Value::as_string)
        .or_else(|| {
            data.get("Input")
                .and_then(Value::as_dictionary)
                .and_then(|d| d.get("IDENTIFIER"))
                .and_then(Value::as_string)
        })
        .ok_or("Trusted parent recipe requires an identifier")?;
    if graph.contains_key(identifier) {
        return Err(format!(
            "Duplicate parent recipe identifier in trust graph: {identifier}"
        ));
    }
    let manifest = autopkg_processors::contract();
    if let Some(process) = data.get("Process") {
        for step in process.as_array().ok_or("Process must be an array")? {
            let step = step.as_dictionary().ok_or("Invalid processor step")?;
            let name = step
                .get("Processor")
                .and_then(Value::as_string)
                .ok_or("Missing processor name")?;
            if manifest["processors"].get(name).is_none() {
                return Err(format!("Custom processor trust is unsupported: {name}"));
            }
        }
    }
    graph.insert(
        identifier.into(),
        (path.clone(), format!("{:x}", Sha256::digest(bytes))),
    );
    if let Some(parent) = data.get("ParentRecipe").or_else(|| data.get("Recipe")) {
        let mut search = dirs.to_vec();
        search.push(path.parent().unwrap().to_owned());
        let parent = locate(
            parent.as_string().ok_or("ParentRecipe must be a string")?,
            &search,
        )?;
        collect(&parent, &search, active, graph)?;
    }
    active.remove(&path);
    Ok(())
}

/// Return reference-format hashes for a parent chain. Paths and git revisions
/// are informational; verification compares identifiers and SHA-256 hashes.
pub fn parent_trust_info(parent: &Path, search_paths: &[PathBuf]) -> Result<Dictionary, String> {
    let mut graph = BTreeMap::new();
    collect(parent, search_paths, &mut HashSet::new(), &mut graph)?;
    let scripts = script_trust(parent, search_paths, &graph)?;
    let processors = processor_trust(parent, search_paths, &graph)?;
    let parents = graph
        .into_iter()
        .map(|(id, (path, hash))| (id, Value::Dictionary(trust_entry(&path, hash))))
        .collect::<Dictionary>();
    Ok(Dictionary::from_iter([
        ("parent_recipes", Value::Dictionary(parents)),
        ("non_core_processors", Value::Dictionary(processors)),
        ("scripts", Value::Dictionary(scripts)),
    ]))
}

/// Preserve the Python source records even though these processors execute Rust.
/// The source is only read as data; neither discovery nor hashing imports it.
fn processor_trust(
    parent: &Path,
    dirs: &[PathBuf],
    graph: &BTreeMap<String, (PathBuf, String)>,
) -> Result<Dictionary, String> {
    let recipe = load_inner(parent, dirs, &mut HashSet::new(), false)?;
    let recipe_dir = recipe.source.parent().ok_or("Recipe has no directory")?;
    let mut processors = Dictionary::new();
    for step in &recipe.process {
        let name = &step.processor;
        if autopkg_processors::community_source(name).is_none() {
            // collect() has already rejected unknown processors. Original core
            // processors never acquire non_core_processors records.
            continue;
        }
        if processors.contains_key(name) {
            continue;
        }
        let mut roots = vec![recipe_dir.to_owned()];
        if let Some((shared_recipe, _)) = name.rsplit_once('/') {
            if let Ok(path) = locate(shared_recipe, dirs) {
                if let Some(directory) = path.parent() {
                    roots.push(directory.to_owned());
                }
            }
        }
        roots.extend(
            graph
                .values()
                .filter_map(|(path, _)| path.parent().map(Path::to_owned)),
        );
        let filename = format!("{}.py", autopkg_processors::canonical_name(name));
        let source = roots
            .into_iter()
            .map(|root| root.join(&filename))
            .find(|path| path.exists());
        let entry = if let Some(path) = source {
            if !path.is_file() {
                return Err(format!(
                    "Processor {name} source is not a regular file: {}",
                    path.display()
                ));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            trust_entry(&path, format!("{:x}", Sha256::digest(bytes)))
        } else {
            // This sentinel and empty path match Python's persisted record.
            // Verification below deliberately refuses to trust a missing file.
            Dictionary::from_iter([
                ("path", ""),
                ("sha256_hash", "PROCESSOR FILEPATH NOT FOUND"),
            ])
        };
        processors.insert(name.clone(), entry.into());
    }
    Ok(processors)
}

fn script_trust(
    parent: &Path,
    dirs: &[PathBuf],
    graph: &BTreeMap<String, (PathBuf, String)>,
) -> Result<Dictionary, String> {
    let recipe = load_inner(parent, dirs, &mut HashSet::new(), false)?;
    let mut roots = vec![recipe
        .source
        .parent()
        .ok_or("Recipe has no directory")?
        .to_owned()];
    roots.extend(
        graph
            .values()
            .filter_map(|(path, _)| path.parent().map(Path::to_owned)),
    );
    let mut scripts = Dictionary::new();
    for step in recipe
        .process
        .iter()
        .filter(|step| step.processor == "PkgCreator")
    {
        let Some(name) = step
            .arguments
            .get("pkg_request")
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("scripts"))
            .and_then(Value::as_string)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(root) = roots
            .iter()
            .map(|root| root.join(name))
            .find(|path| path.is_dir())
        else {
            continue;
        };
        let tracked = std::process::Command::new("git")
            .args(["ls-files", "-z"])
            .current_dir(&root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .split('\0')
                    .filter(|s| !s.is_empty())
                    .map(|s| root.join(s))
                    .collect::<HashSet<_>>()
            });
        let mut files = Vec::new();
        script_files(&root, &mut files)?;
        for path in files {
            if tracked
                .as_ref()
                .is_some_and(|tracked| !tracked.contains(&path))
            {
                continue;
            }
            let key = normalized_script_directory(name)
                .join(path.strip_prefix(&root).map_err(|e| e.to_string())?)
                .to_string_lossy()
                .replace('\\', "/");
            if scripts.contains_key(&key) {
                continue;
            }
            let hash = if path.is_file() {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                format!("{:x}", Sha256::digest(bytes))
            } else {
                "NOT A FILE".into()
            };
            scripts.insert(
                key,
                Value::Dictionary(trust_entry(
                    &normalized_script_directory(&path.to_string_lossy()),
                    hash,
                )),
            );
        }
    }
    Ok(scripts)
}

fn script_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            script_files(&entry.path(), files)?;
        } else if !entry.path().is_dir() {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn normalized_script_directory(name: &str) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir if normalized.file_name().is_some_and(|last| last != "..") => {
                normalized.pop();
            }
            Component::ParentDir if normalized.has_root() => (),
            component => normalized.push(component.as_os_str()),
        }
    }
    if normalized.as_os_str().is_empty() {
        normalized.push(".");
    }
    normalized
}

fn trust_entry(path: &Path, hash: String) -> Dictionary {
    let display_path = crate::normal_recipe_path(path.to_owned());
    let display = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .and_then(|home| {
            display_path
                .strip_prefix(PathBuf::from(home))
                .ok()
                .map(|relative| Path::new("~").join(relative))
        })
        .unwrap_or(display_path);
    let mut entry = Dictionary::from_iter([
        (
            "path",
            Value::String(display.to_string_lossy().into_owned()),
        ),
        ("sha256_hash", Value::String(hash)),
    ]);
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(output) = std::process::Command::new("git")
            .args(["rev-list", "-1", "HEAD", "--"])
            .arg(name)
            .current_dir(parent)
            .output()
        {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let clean = !hash.is_empty()
                && std::process::Command::new("git")
                    .args(["diff", &hash, "--"])
                    .arg(name)
                    .current_dir(parent)
                    .output()
                    .is_ok_and(|diff| diff.status.success() && diff.stdout.is_empty());
            if output.status.success() && clean {
                entry.insert("git_hash".into(), hash.into());
            }
        }
    }
    entry
}

fn verify(expected: &Dictionary, actual: &Dictionary) -> Result<(), String> {
    let processors = expected
        .get("non_core_processors")
        .ok_or("Missing non_core_processors trust record")?
        .as_dictionary()
        .ok_or("Invalid trust record dictionary")?;
    let empty = Dictionary::new();
    let actual_processors = actual
        .get("non_core_processors")
        .map(|v| {
            v.as_dictionary()
                .ok_or("Invalid non_core_processors trust dictionary")
        })
        .transpose()?
        .unwrap_or(&empty);
    for name in processors.keys().chain(actual_processors.keys()) {
        if autopkg_processors::community_source(name).is_none() {
            return Err(format!("Custom processor trust is unsupported: {name}"));
        }
    }
    if processors.len() != actual_processors.len()
        || processors
            .keys()
            .any(|name| !actual_processors.contains_key(name))
    {
        return Err("Non-core processor trust graph differs from expected".into());
    }
    for (name, entry) in processors {
        let hash = entry
            .as_dictionary()
            .and_then(|entry| entry.get("sha256_hash"))
            .and_then(Value::as_string)
            .ok_or("Invalid processor SHA-256 trust record")?;
        let actual_hash = actual_processors[name]
            .as_dictionary()
            .and_then(|entry| entry.get("sha256_hash"))
            .and_then(Value::as_string)
            .ok_or("Invalid processor SHA-256 trust record")?;
        if actual_hash == "PROCESSOR FILEPATH NOT FOUND" {
            return Err(format!("Expected processor {name} can't be found"));
        }
        if hash != actual_hash {
            return Err(format!("Processor {name} contents differ from expected"));
        }
    }
    let expected_scripts = expected
        .get("scripts")
        .map(|v| v.as_dictionary().ok_or("Invalid scripts trust dictionary"))
        .transpose()?
        .unwrap_or(&empty);
    let actual_scripts = actual
        .get("scripts")
        .and_then(Value::as_dictionary)
        .unwrap_or(&empty);
    for (key, entry) in expected_scripts {
        let hash = entry
            .as_dictionary()
            .and_then(|d| d.get("sha256_hash"))
            .and_then(Value::as_string)
            .ok_or("Invalid script SHA-256 trust record")?;
        let actual_hash = actual_scripts
            .get(key)
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("sha256_hash"))
            .and_then(Value::as_string);
        if actual_hash != Some(hash) {
            return Err(format!(
                "Script {key} contents differ from expected or cannot be found"
            ));
        }
    }
    for key in actual_scripts
        .keys()
        .filter(|key| !expected_scripts.contains_key(key))
    {
        eprintln!("WARNING: Script trust info missing for {key}. Run autopkg update-trust-info to add script trust info. Starting in AutoPkg 3.1.0, this will be a trust verification error.");
    }
    let expected = expected
        .get("parent_recipes")
        .and_then(Value::as_dictionary)
        .ok_or("Missing parent_recipes trust record")?;
    let actual = actual["parent_recipes"].as_dictionary().unwrap();
    if expected.len() != actual.len() || expected.keys().any(|k| !actual.contains_key(k)) {
        return Err("Parent recipe trust graph differs from expected".into());
    }
    for (id, entry) in expected {
        let hash = entry
            .as_dictionary()
            .and_then(|d| d.get("sha256_hash"))
            .and_then(Value::as_string)
            .ok_or("Invalid parent recipe SHA-256 trust record")?;
        if actual[id].as_dictionary().unwrap()["sha256_hash"].as_string() != Some(hash) {
            return Err(format!("Parent recipe {id} contents differ from expected"));
        }
    }
    Ok(())
}

/// Load a trusted local override. The caller supplies configured directories;
/// repository origins take precedence over override-directory membership.
pub fn load_verified_recipe(
    path: &Path,
    search_paths: &[PathBuf],
    policy: &TrustPolicy,
) -> Result<Recipe, String> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !is_under(&path, &policy.override_dirs) || is_under(&path, &policy.repository_dirs) {
        return Err(
            "Trust records must come from a local override outside recipe repositories".into(),
        );
    }
    let original = std::fs::read(&path).map_err(|e| e.to_string())?;
    let data = read_recipe(&path)?;
    let expected = data
        .get("ParentRecipeTrustInfo")
        .and_then(Value::as_dictionary)
        .ok_or("No parent recipe trust information present")?;
    let parent = data
        .get("ParentRecipe")
        .or_else(|| data.get("Recipe"))
        .and_then(Value::as_string)
        .ok_or("Trusted override requires ParentRecipe")?;
    let mut dirs = search_paths.to_vec();
    dirs.push(path.parent().unwrap().to_owned());
    let parent = locate(parent, &dirs)?;
    let actual = parent_trust_info(&parent, &dirs)?;
    verify(expected, &actual)?;
    let recipe = load_inner(&path, search_paths, &mut HashSet::new(), true)?;
    // Check again after loading so concurrent ordinary edits cannot silently
    // replace the contents between graph verification and recipe parsing.
    verify(expected, &parent_trust_info(&parent, &dirs)?)?;
    if std::fs::read(&path).map_err(|e| e.to_string())? != original {
        return Err("Override changed while verifying trust".into());
    }
    Ok(recipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn community_parent(root: &Path, name: &str) -> PathBuf {
        let parent = root.join("parent.recipe.yaml");
        std::fs::write(
            &parent,
            format!("Identifier: org.parent\nInput: {{}}\nProcess:\n- Processor: {name}\n"),
        )
        .unwrap();
        parent
    }

    #[test]
    fn promoted_processor_trust_preserves_python_records_and_detects_changes() {
        let temp = tempfile::tempdir().unwrap();
        let parent = community_parent(temp.path(), "MozillaURLProvider");
        let source = temp.path().join("MozillaURLProvider.py");
        // Invalid Python demonstrates that trust hashes bytes without importing.
        std::fs::write(&source, "this must never execute!").unwrap();
        let info = parent_trust_info(&parent, &[]).unwrap();
        let processors = info["non_core_processors"].as_dictionary().unwrap();
        assert_eq!(processors.len(), 1);
        assert_eq!(
            processors["MozillaURLProvider"].as_dictionary().unwrap()["sha256_hash"].as_string(),
            Some(format!("{:x}", Sha256::digest(b"this must never execute!")).as_str())
        );
        verify(&info, &info).unwrap();
        std::fs::write(&source, "changed").unwrap();
        assert!(verify(&info, &parent_trust_info(&parent, &[]).unwrap())
            .unwrap_err()
            .contains("Processor MozillaURLProvider contents differ"));
        std::fs::remove_file(source).unwrap();
        let missing = parent_trust_info(&parent, &[]).unwrap();
        let entry = missing["non_core_processors"].as_dictionary().unwrap()["MozillaURLProvider"]
            .as_dictionary()
            .unwrap();
        assert_eq!(entry["path"].as_string(), Some(""));
        assert_eq!(
            entry["sha256_hash"].as_string(),
            Some("PROCESSOR FILEPATH NOT FOUND")
        );
        assert!(verify(&info, &missing)
            .unwrap_err()
            .contains("can't be found"));
        assert!(verify(&missing, &missing)
            .unwrap_err()
            .contains("can't be found"));
        // Missing legacy source files are relevant only to trust verification.
        crate::load_recipe(&parent, &[]).unwrap();
    }

    #[test]
    fn promoted_processor_source_search_prefers_recipe_then_shared_then_parent() {
        let temp = tempfile::tempdir().unwrap();
        let child_dir = temp.path().join("child");
        let base_dir = temp.path().join("base");
        let shared_dir = temp.path().join("shared");
        for dir in [&child_dir, &base_dir, &shared_dir] {
            std::fs::create_dir(dir).unwrap();
        }
        let alias = "com.github.autopkg.AutoPkgGitMaster/GenerateRelocatablePython";
        let base = community_parent(&base_dir, alias);
        let child = child_dir.join("child.recipe.yaml");
        std::fs::write(
            &child,
            "Identifier: org.child\nParentRecipe: org.parent\nInput: {}\nProcess: []\n",
        )
        .unwrap();
        let shared = shared_dir.join("shared.recipe.yaml");
        std::fs::write(
            &shared,
            "Identifier: com.github.autopkg.AutoPkgGitMaster\nInput: {}\nProcess: []\n",
        )
        .unwrap();
        let filename = "GenerateRelocatablePython.py";
        for (dir, value) in [
            (&child_dir, "child"),
            (&base_dir, "base"),
            (&shared_dir, "shared"),
        ] {
            std::fs::write(dir.join(filename), value).unwrap();
        }
        let dirs = [temp.path().to_owned()];
        let assert_source = |expected: &str| {
            let info = parent_trust_info(&child, &dirs).unwrap();
            let entries = info["non_core_processors"].as_dictionary().unwrap();
            assert!(entries.contains_key(alias));
            assert!(!entries.contains_key("GenerateRelocatablePython"));
            assert_eq!(
                entries[alias].as_dictionary().unwrap()["sha256_hash"].as_string(),
                Some(format!("{:x}", Sha256::digest(expected.as_bytes())).as_str())
            );
            verify(&info, &info).unwrap();
        };
        assert_source("child");
        std::fs::remove_file(child_dir.join(filename)).unwrap();
        assert_source("shared");
        std::fs::remove_file(shared_dir.join(filename)).unwrap();
        assert_source("base");
        assert!(base.is_file());
    }

    #[test]
    fn promoted_processor_trust_requires_exact_name_sets_and_rejects_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let parent = community_parent(temp.path(), "MozillaURLProvider");
        std::fs::write(temp.path().join("MozillaURLProvider.py"), "source").unwrap();
        let actual = parent_trust_info(&parent, &[]).unwrap();
        let mut missing = actual.clone();
        missing.insert("non_core_processors".into(), Dictionary::new().into());
        assert!(verify(&missing, &actual)
            .unwrap_err()
            .contains("graph differs"));
        assert!(verify(&actual, &missing)
            .unwrap_err()
            .contains("graph differs"));
        let mut unknown = actual.clone();
        unknown
            .get_mut("non_core_processors")
            .unwrap()
            .as_dictionary_mut()
            .unwrap()
            .insert(
                "UnknownProcessor".into(),
                Dictionary::from_iter([("sha256_hash", "hash")]).into(),
            );
        assert!(verify(&unknown, &actual)
            .unwrap_err()
            .contains("Custom processor trust is unsupported"));
        community_parent(temp.path(), "evil.identifier/MozillaURLProvider");
        assert!(parent_trust_info(&parent, &[])
            .unwrap_err()
            .contains("Custom processor trust is unsupported"));
    }

    #[test]
    fn promoted_processor_trust_loads_existing_override_without_executing_source() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let overrides = temp.path().join("overrides");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&overrides).unwrap();
        let parent = community_parent(&repo, "MozillaURLProvider");
        std::fs::write(
            repo.join("MozillaURLProvider.py"),
            "raise RuntimeError('do not import')",
        )
        .unwrap();
        let info = parent_trust_info(&parent, &[]).unwrap();
        let child = overrides.join("child.recipe");
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::from("org.child")),
            ("Input", Dictionary::new().into()),
            ("ParentRecipe", Value::from("org.parent")),
            ("ParentRecipeTrustInfo", info.into()),
        ]))
        .to_file_xml(&child)
        .unwrap();
        let policy = TrustPolicy {
            override_dirs: vec![overrides],
            repository_dirs: vec![repo.clone()],
        };
        assert_eq!(
            load_verified_recipe(&child, &[repo], &policy)
                .unwrap()
                .identifier,
            "org.child"
        );
    }

    #[cfg(unix)]
    #[test]
    fn nonregular_script_child() {
        let Some(root) = std::env::var_os("AUTOPKG_TRUST_FIFO_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        let trust = parent_trust_info(&root.join("parent.recipe.yaml"), &[]).unwrap();
        let scripts = trust["scripts"].as_dictionary().unwrap();
        for name in ["scripts/fifo", "scripts/broken"] {
            assert_eq!(
                scripts[name].as_dictionary().unwrap()["sha256_hash"].as_string(),
                Some("NOT A FILE")
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn nonregular_script_hashing_does_not_block_or_dereference_broken_links() {
        use std::{
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("scripts")).unwrap();
        std::fs::write(root.join("parent.recipe.yaml"), "Identifier: org.parent\nInput: {}\nProcess:\n- Processor: PkgCreator\n  Arguments:\n    pkg_request:\n      scripts: scripts\n").unwrap();
        assert!(Command::new("mkfifo")
            .arg(root.join("scripts/fifo"))
            .status()
            .unwrap()
            .success());
        std::os::unix::fs::symlink("missing", root.join("scripts/broken")).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "trust::tests::nonregular_script_child",
                "--nocapture",
            ])
            .env("AUTOPKG_TRUST_FIFO_FIXTURE", root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Trust hashing blocked on a nonregular script file");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn dirty_files_do_not_claim_the_last_commit_as_their_revision() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        let script = root.join("postinstall");
        std::fs::write(&script, "original").unwrap();
        git(&["add", "postinstall"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
        ]);
        assert!(trust_entry(&script, "hash".into()).contains_key("git_hash"));
        std::fs::write(&script, "locally modified").unwrap();
        assert!(!trust_entry(&script, "hash".into()).contains_key("git_hash"));
    }
    #[test]
    fn verifies_python_normalized_script_keys_in_existing_override() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let overrides = temp.path().join("overrides");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::create_dir(&overrides).unwrap();
        let parent = repo.join("parent.recipe.yaml");
        let source = b"Identifier: org.parent\nInput: {}\nProcess:\n- Processor: PkgCreator\n  Arguments:\n    pkg_request:\n      scripts: ./scripts\n";
        std::fs::write(&parent, source).unwrap();
        let script = repo.join("scripts/postinstall");
        std::fs::write(&script, b"original").unwrap();
        // This is the persisted Python shape: normpath('./scripts') becomes
        // 'scripts', regardless of the spelling in the parent recipe.
        let expected = Dictionary::from_iter([
            (
                "parent_recipes",
                Value::Dictionary(Dictionary::from_iter([(
                    "org.parent",
                    Value::Dictionary(trust_entry(
                        &parent,
                        format!("{:x}", Sha256::digest(source)),
                    )),
                )])),
            ),
            ("non_core_processors", Value::Dictionary(Dictionary::new())),
            (
                "scripts",
                Value::Dictionary(Dictionary::from_iter([(
                    "scripts/postinstall",
                    Value::Dictionary(trust_entry(
                        &script,
                        format!("{:x}", Sha256::digest(b"original")),
                    )),
                )])),
            ),
        ]);
        let child = overrides.join("child.recipe");
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::from("org.child")),
            ("Input", Value::Dictionary(Dictionary::new())),
            ("ParentRecipe", Value::from("org.parent")),
            ("ParentRecipeTrustInfo", Value::Dictionary(expected)),
        ]))
        .to_file_xml(&child)
        .unwrap();
        let policy = TrustPolicy {
            override_dirs: vec![overrides],
            repository_dirs: vec![repo.clone()],
        };
        assert_eq!(
            load_verified_recipe(&child, &[repo], &policy)
                .unwrap()
                .identifier,
            "org.child"
        );
    }
    #[test]
    fn script_trust_tracks_git_files_and_preserves_reference_new_script_warning() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        let parent = root.join("parent.recipe.yaml");
        std::fs::write(&parent,"Identifier: org.parent\nInput: {}\nProcess:\n- Processor: PkgCreator\n  Arguments:\n    pkg_request:\n      scripts: scripts\n").unwrap();
        std::fs::create_dir(root.join("scripts")).unwrap();
        std::fs::write(root.join("scripts/postinstall"), "original").unwrap();
        std::fs::write(root.join("scripts/untracked"), "untracked").unwrap();
        git(&["add", "parent.recipe.yaml", "scripts/postinstall"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ]);
        let original = parent_trust_info(&parent, &[]).unwrap();
        let scripts = original["scripts"].as_dictionary().unwrap();
        assert_eq!(scripts.len(), 1);
        assert!(scripts["scripts/postinstall"]
            .as_dictionary()
            .unwrap()
            .contains_key("git_hash"));
        git(&["add", "scripts/untracked"]);
        let updated = parent_trust_info(&parent, &[]).unwrap();
        verify(&original, &updated).unwrap();
        std::fs::remove_file(root.join("scripts/postinstall")).unwrap();
        assert!(verify(&original, &parent_trust_info(&parent, &[]).unwrap())
            .unwrap_err()
            .contains("Script scripts/postinstall"));
    }
    #[test]
    fn verifies_parents_and_rejects_mutation_and_external_origin() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let overrides = temp.path().join("overrides");
        std::fs::create_dir(&repo).unwrap();
        std::fs::create_dir(&overrides).unwrap();
        let parent = repo.join("parent.recipe.yaml");
        std::fs::write(&parent, "Identifier: org.parent\nInput: {}\nProcess: []\n").unwrap();
        let info = parent_trust_info(&parent, &[]).unwrap();
        let child = overrides.join("child.recipe");
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::String("org.child".into())),
            ("Input", Value::Dictionary(Dictionary::new())),
            ("ParentRecipe", Value::String("org.parent".into())),
            ("ParentRecipeTrustInfo", Value::Dictionary(info)),
        ]))
        .to_file_xml(&child)
        .unwrap();
        let policy = TrustPolicy {
            override_dirs: vec![overrides.clone()],
            repository_dirs: vec![repo.clone()],
        };
        assert_eq!(
            load_verified_recipe(&child, std::slice::from_ref(&repo), &policy)
                .unwrap()
                .identifier,
            "org.child"
        );
        let bad = TrustPolicy {
            override_dirs: vec![temp.path().to_owned()],
            repository_dirs: vec![overrides],
        };
        assert!(
            load_verified_recipe(&child, std::slice::from_ref(&repo), &bad)
                .unwrap_err()
                .contains("local override")
        );
        std::fs::write(
            &parent,
            "Identifier: org.parent\nInput: {NAME: changed}\nProcess: []\n",
        )
        .unwrap();
        assert!(load_verified_recipe(&child, &[repo], &policy)
            .unwrap_err()
            .contains("contents differ"));
    }
    #[test]
    fn graph_detects_new_parents() {
        let expected = Dictionary::from_iter([
            ("parent_recipes", Value::Dictionary(Dictionary::new())),
            ("non_core_processors", Value::Dictionary(Dictionary::new())),
        ]);
        let actual = Dictionary::from_iter([(
            "parent_recipes",
            Value::Dictionary(Dictionary::from_iter([(
                "extra",
                Value::String("bad".into()),
            )])),
        )]);
        assert!(verify(&expected, &actual).unwrap_err().contains("graph"));
    }
    #[test]
    fn hashes_entire_inheritance_chain_and_script_contents() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent.recipe.yaml");
        let grandparent = temp.path().join("base.recipe.yaml");
        std::fs::write(
            &grandparent,
            "Identifier: org.base\nInput: {}\nProcess: []\n",
        )
        .unwrap();
        std::fs::write(
            &parent,
            "Identifier: org.parent\nParentRecipe: org.base\nInput: {}\nProcess: []\n",
        )
        .unwrap();
        let info = parent_trust_info(&parent, &[]).unwrap();
        assert_eq!(info["parent_recipes"].as_dictionary().unwrap().len(), 2);
        std::fs::write(
            &grandparent,
            "Identifier: org.base\nInput: {changed: true}\nProcess: []\n",
        )
        .unwrap();
        assert!(verify(&info, &parent_trust_info(&parent, &[]).unwrap())
            .unwrap_err()
            .contains("org.base"));
        std::fs::write(&parent,"Identifier: org.parent\nInput: {}\nProcess:\n- Processor: PkgCreator\n  Arguments:\n    pkg_request:\n      scripts: scripts\n").unwrap();
        let scripts = temp.path().join("scripts");
        std::fs::create_dir(&scripts).unwrap();
        std::fs::write(scripts.join("postinstall"), b"original").unwrap();
        let expected = parent_trust_info(&parent, &[]).unwrap();
        assert!(expected["scripts"]
            .as_dictionary()
            .unwrap()
            .contains_key("scripts/postinstall"));
        std::fs::write(scripts.join("postinstall"), b"changed").unwrap();
        assert!(verify(&expected, &parent_trust_info(&parent, &[]).unwrap())
            .unwrap_err()
            .contains("Script scripts/postinstall"));
    }
}
