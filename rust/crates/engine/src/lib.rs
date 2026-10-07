//! Sequential recipe loading and execution. No Python processors are loaded.
use plist::python_repr;
pub mod cache;
pub mod preferences;
pub mod report;
pub mod trust;
mod yaml;
use plist::{Dictionary, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Step {
    pub processor: String,
    pub arguments: Dictionary,
}
#[derive(Clone, Debug)]
pub struct Recipe {
    pub identifier: String,
    pub input: Dictionary,
    pub process: Vec<Step>,
    pub source: PathBuf,
}
impl Recipe {
    /// Python's CLI retains the process through its last check-phase marker.
    pub fn check_phase(&self) -> Result<Self, String> {
        let last = self.process.iter().rposition(|step| step.processor == "EndOfCheckPhase")
            .ok_or_else(|| format!("Recipe at {} is missing EndOfCheckPhase Processor, not possible to perform check.", self.source.display()))?;
        let mut recipe = self.clone();
        recipe.process.truncate(last + 1);
        Ok(recipe)
    }
    /// Add command-line processors around the fully inherited process sequence.
    pub fn add_processors(&mut self, pre: &[String], post: &[String]) {
        let step = |name: &String| Step {
            processor: name.clone(),
            arguments: Dictionary::new(),
        };
        self.process.splice(0..0, pre.iter().map(step));
        self.process.extend(post.iter().map(step));
    }
}
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub check_only: bool,
    /// Immutable application preferences, distinct from recipe/CLI environment values.
    pub preferences: Option<Dictionary>,
}
#[derive(Clone, Debug)]
pub struct RunResult {
    pub environment: Dictionary,
    pub executed: Vec<String>,
    pub stopped: bool,
    pub receipt: Vec<Value>,
}
#[derive(Clone, Debug)]
pub struct RunFailure {
    pub message: String,
    pub partial: Box<RunResult>,
}
impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for RunFailure {}

fn yaml_value(v: serde_yaml::Value) -> Result<Value, String> {
    use serde_yaml::Value as Y;
    Ok(match v {
        Y::Null => Value::Null,
        Y::Bool(v) => Value::Boolean(v),
        Y::Number(v) => {
            if let Some(n) = v.as_i64() {
                Value::Integer(n.into())
            } else if let Some(n) = v.as_u64() {
                Value::Integer(n.into())
            } else {
                Value::Real(v.as_f64().ok_or("Invalid YAML number")?)
            }
        }
        Y::String(v) => Value::String(v),
        Y::Sequence(v) => Value::Array(v.into_iter().map(yaml_value).collect::<Result<_, _>>()?),
        Y::Mapping(v) => Value::Dictionary(
            v.into_iter()
                .map(|(k, v)| {
                    Ok((
                        k.as_str()
                            .ok_or("Recipe dictionary keys must be strings")?
                            .to_owned(),
                        yaml_value(v)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        ),
        Y::Tagged(_) => return Err("Tagged YAML values are not yet supported".into()),
    })
}
/// Serialize a recipe while retaining YAML scalar types and null values.
pub fn to_recipe_yaml(value: &Value) -> Result<String, String> {
    yaml::serialize(value)
}

pub fn read_recipe(path: &Path) -> Result<Dictionary, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = if matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("yaml" | "yml")
    ) {
        yaml::parse(&bytes)?
    } else {
        Value::from_reader(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?
    };
    value
        .into_dictionary()
        .ok_or_else(|| "Recipe must be a dictionary".into())
}
const RECIPE_EXTENSIONS: [&str; 3] = [".recipe", ".recipe.yaml", ".recipe.plist"];
/// Real directories directly below `dir`. Symbolic links are skipped, so a
/// repository can't redirect a lookup elsewhere.
fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}
/// Regular recipe files directly inside `dir`, sorted by path.
fn recipe_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry.file_name().to_str().is_some_and(|name| {
                RECIPE_EXTENSIONS
                    .iter()
                    .any(|extension| name.ends_with(extension))
            })
        })
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}
/// Recipe files at the top level of `dir` and then one folder down, like the
/// reference's `*.recipe` and `*/*.recipe` globs. The scan stays shallow, so a
/// broad search folder such as `.` in a home folder or `/` stays cheap, and
/// unreadable folders are skipped.
fn candidates(dir: &Path) -> Vec<PathBuf> {
    let mut files = recipe_files(dir);
    for subdirectory in subdirectories(dir) {
        files.extend(recipe_files(&subdirectory));
    }
    files
}
fn locate(name: &str, dirs: &[PathBuf]) -> Result<PathBuf, String> {
    // Like the reference, search each folder's top level and then one folder
    // down before moving to the next folder; recipe repositories keep recipes
    // in per-vendor subfolders.
    let nested_names: Vec<String> = if RECIPE_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
        vec![name.to_owned()]
    } else {
        RECIPE_EXTENSIONS
            .iter()
            .map(|ext| format!("{name}{ext}"))
            .collect()
    };
    for dir in dirs {
        for suffix in [""].into_iter().chain(RECIPE_EXTENSIONS) {
            let p = dir.join(format!("{name}{suffix}"));
            if p.is_file() {
                return Ok(p);
            }
        }
        let subdirectories = subdirectories(dir);
        for file_name in &nested_names {
            for subdirectory in &subdirectories {
                let p = subdirectory.join(file_name);
                if std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
                    return Ok(p);
                }
            }
        }
    }
    for dir in dirs {
        for path in candidates(dir) {
            if let Ok(d) = read_recipe(&path) {
                if d.get("Identifier").and_then(Value::as_string) == Some(name) {
                    return Ok(path);
                }
            }
        }
    }
    Err(format!("Could not find parent recipe {name}"))
}
pub fn load_recipe(path: &Path, search_paths: &[PathBuf]) -> Result<Recipe, String> {
    let resolved = resolve_recipe(path, search_paths)?;
    load_inner(&resolved, search_paths, &mut HashSet::new(), false)
}
/// Explicit CLI escape hatch; execution still validates built-ins and inputs.
/// Call only when the user requests ignoring parent trust verification errors.
pub fn load_unverified_recipe(path: &Path, search_paths: &[PathBuf]) -> Result<Recipe, String> {
    let resolved = resolve_recipe(path, search_paths)?;
    load_inner(&resolved, search_paths, &mut HashSet::new(), true)
}
pub fn resolve_recipe(path: &Path, search_paths: &[PathBuf]) -> Result<PathBuf, String> {
    let resolved = if path.is_file() {
        path.to_owned()
    } else {
        let mut dirs = vec![PathBuf::from(".")];
        dirs.extend_from_slice(search_paths);
        locate(&path.to_string_lossy(), &dirs)?
    };
    Ok(resolved)
}
/// Canonical Windows paths exposed to recipe substitutions must retain normal
/// Win32 separator semantics, rather than the verbatim-prefix restrictions.
pub fn normal_recipe_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            let mut normal = match prefix.kind() {
                Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", drive as char)),
                Prefix::VerbatimUNC(server, share) => {
                    let mut value = std::ffi::OsString::from(r"\\");
                    value.push(server);
                    value.push(r"\");
                    value.push(share);
                    PathBuf::from(value)
                }
                _ => return path,
            };
            for component in path.components().skip(1) {
                normal.push(component.as_os_str());
            }
            return normal;
        }
    }
    path
}
fn load_inner(
    path: &Path,
    dirs: &[PathBuf],
    active: &mut HashSet<PathBuf>,
    verified_trust: bool,
) -> Result<Recipe, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let path = normal_recipe_path(path);
    if !active.insert(path.clone()) {
        return Err(format!("Recipe inheritance cycle at {}", path.display()));
    }
    let d = read_recipe(&path)?;
    if let Some(minimum) = d.get("MinimumVersion") {
        let minimum = minimum
            .as_string()
            .ok_or("MinimumVersion must be a string")?;
        if autopkg_platform::github::compare_versions("3.0.0", minimum).is_lt() {
            return Err(format!("Recipe requires at least autopkg version {minimum}, but this implementation targets 3.0.0."));
        }
    }
    if d.contains_key("ParentRecipeTrustInfo") && !verified_trust {
        return Err("Parent recipe trust must be verified before loading this override".into());
    }
    let input = d
        .get("Input")
        .and_then(Value::as_dictionary)
        .ok_or("Recipe requires an Input dictionary")?
        .clone();
    let identifier = d
        .get("Identifier")
        .or_else(|| input.get("IDENTIFIER"))
        .and_then(Value::as_string)
        .map(str::to_owned)
        .unwrap_or_else(|| path.with_extension("").to_string_lossy().replace('/', "-"));
    let mut recipe = if let Some(parent) = d.get("ParentRecipe").or_else(|| d.get("Recipe")) {
        let parent = parent.as_string().ok_or("ParentRecipe must be a string")?;
        let mut search = dirs.to_vec();
        search.push(path.parent().unwrap().to_owned());
        load_inner(&locate(parent, &search)?, &search, active, verified_trust)?
    } else {
        Recipe {
            identifier: identifier.clone(),
            input: Dictionary::new(),
            process: Vec::new(),
            source: path.clone(),
        }
    };
    recipe.identifier = identifier;
    recipe.source = path.clone();
    recipe.input.extend(input);
    if let Some(steps) = d.get("Process") {
        for step in steps.as_array().ok_or("Process must be an array")? {
            let step = step
                .as_dictionary()
                .ok_or("Process entries must be dictionaries")?;
            let processor = step
                .get("Processor")
                .and_then(Value::as_string)
                .ok_or("Process entry requires Processor")?
                .to_owned();
            let arguments = match step.get("Arguments") {
                Some(v) => v
                    .as_dictionary()
                    .ok_or("Arguments must be a dictionary")?
                    .clone(),
                None => Dictionary::new(),
            };
            recipe.process.push(Step {
                processor,
                arguments,
            });
        }
    } else if !d.contains_key("ParentRecipe") && !d.contains_key("Recipe") {
        return Err("Recipe requires a Process array".into());
    }
    active.remove(&path);
    Ok(recipe)
}

fn python_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null | Value::Boolean(false) => String::new(),
        Value::Boolean(true) => "True".into(),
        Value::Integer(n) => n.to_string(),
        Value::Date(date) => date
            .to_xml_format()
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_owned(),
        _ => python_repr(value),
    }
}
fn pretty_repr(value: &Value, indent: usize) -> String {
    let mut sorted = value.clone();
    fn sort(value: &mut Value) {
        match value {
            Value::Dictionary(d) => {
                d.sort_keys();
                for (_, v) in d.iter_mut() {
                    sort(v);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(sort),
            _ => {}
        }
    }
    sort(&mut sorted);
    let value = &sorted;
    let plain = python_repr(value);
    if indent + plain.chars().count() <= 80 {
        return plain;
    }
    match value {
        Value::Dictionary(values) if !values.is_empty() => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            let rows = entries
                .into_iter()
                .map(|(key, value)| {
                    let label = python_repr(&Value::String(key.clone()));
                    format!(
                        "{label}: {}",
                        pretty_repr(value, indent + 1 + label.chars().count() + 2)
                    )
                })
                .collect::<Vec<_>>();
            format!(
                "{{{}}}",
                rows.join(&format!(",\n{}", " ".repeat(indent + 1)))
            )
        }
        Value::Array(values) if values.len() > 1 => format!(
            "[{}]",
            values
                .iter()
                .map(|v| pretty_repr(v, indent + 1))
                .collect::<Vec<_>>()
                .join(&format!(",\n{}", " ".repeat(indent + 1)))
        ),
        _ => plain,
    }
}

/// Match the reference's single-pass substitution, including unchanged missing keys.
pub fn substitute(value: &Value, env: &Dictionary) -> Result<Value, String> {
    Ok(match value {
        Value::String(s) => {
            let re = regex::Regex::new(r"%([a-zA-Z_][a-zA-Z_0-9]*)%").unwrap();
            if re.captures_iter(s).any(|c| !env.contains_key(&c[1])) {
                return Ok(value.clone());
            }
            Value::String(
                re.replace_all(s, |c: &regex::Captures<'_>| python_string(&env[&c[1]]))
                    .into_owned(),
            )
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| substitute(v, env))
                .collect::<Result<_, _>>()?,
        ),
        Value::Dictionary(d) => Value::Dictionary(
            d.iter()
                .map(|(k, v)| Ok((k.clone(), substitute(v, env)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => value.clone(),
    })
}
fn reject_backend(d: &Dictionary) -> Result<(), String> {
    if let Some(plugin) = d.get("MUNKI_REPO_PLUGIN") {
        if plugin.as_string() != Some("FileRepo") {
            return Err("Only Munki FileRepo repositories are supported".into());
        }
    }
    if d.get("force_munki_repo_lib").is_some_and(truthy) {
        return Err("Python Munki repository libraries are not supported".into());
    }
    for key in ["MUNKI_REPO", "munki_repo"] {
        if let Some(s) = d.get(key).and_then(Value::as_string) {
            if s.contains("://") && !s.starts_with("file://") {
                return Err("Only Munki FileRepo repositories are supported".into());
            }
        }
    }
    Ok(())
}
pub fn validate_recipe(recipe: &Recipe, env: &Dictionary) -> Result<(), String> {
    reject_backend(&recipe.input)?;
    reject_backend(env)?;
    let manifests = autopkg_processors::contract();
    let mut variables: HashSet<String> = recipe.input.keys().chain(env.keys()).cloned().collect();
    let mut preview = recipe.input.clone();
    preview.extend(env.clone());
    for (key, value) in preview.clone() {
        let value = substitute(&value, &preview)?;
        preview.insert(key, value);
    }
    reject_backend(&preview)?;
    for step in &recipe.process {
        if !autopkg_processors::supported().contains(&step.processor.as_str()) {
            if manifests["processors"].get(&step.processor).is_some() {
                return Err(format!(
                    "Built-in processor '{}' is not yet implemented",
                    step.processor
                ));
            }
            return Err(format!(
                "Custom or unknown processor '{}' is not supported",
                step.processor
            ));
        }
        reject_backend(&step.arguments)?;
        for (key, value) in &step.arguments {
            let value = substitute(value, &preview)?;
            preview.insert(key.clone(), value);
        }
        reject_backend(&preview)?;
        variables.extend(step.arguments.keys().cloned());
        let manifest = &manifests["processors"][&step.processor];
        if let Some(inputs) = manifest["input_variables"].as_object() {
            for (key, flags) in inputs {
                if flags["required"].as_bool() == Some(true) && !variables.contains(key) {
                    return Err(format!(
                        "{} requires missing argument {key}",
                        step.processor
                    ));
                }
            }
        }
        if let Some(outputs) = manifest["output_variables"].as_object() {
            variables.extend(outputs.keys().cloned());
        }
    }
    Ok(())
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Boolean(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Integer(v) => v.as_signed() != Some(0),
        Value::Real(v) => *v != 0.0,
        Value::Data(v) => !v.is_empty(),
        Value::Array(v) => !v.is_empty(),
        Value::Dictionary(v) => !v.is_empty(),
        _ => true,
    }
}
pub fn run_recipe(
    recipe: &Recipe,
    initial: Dictionary,
    options: &RunOptions,
) -> Result<RunResult, String> {
    run_recipe_detailed(recipe, initial, options).map_err(|failure| failure.message)
}
pub fn run_recipe_detailed(
    recipe: &Recipe,
    initial: Dictionary,
    options: &RunOptions,
) -> Result<RunResult, RunFailure> {
    let _images = autopkg_platform::dmg::RecipeScope::new();
    if options.check_only {
        return match recipe.check_phase() {
            Ok(recipe) => run_recipe_detailed(
                &recipe,
                initial,
                &RunOptions {
                    check_only: false,
                    preferences: options.preferences.clone(),
                },
            ),
            Err(message) => Err(RunFailure {
                message,
                partial: Box::new(RunResult {
                    environment: initial,
                    executed: vec![],
                    stopped: false,
                    receipt: vec![],
                }),
            }),
        };
    }
    let mut env = recipe.input.clone();
    env.extend(initial);
    let mut executed = Vec::new();
    let mut stopped = false;
    let mut receipt = Vec::new();
    let outcome = (|| -> Result<(), String> {
        validate_recipe(recipe, &env)?;
        env.insert(
            "RECIPE_PATH".into(),
            Value::String(recipe.source.to_string_lossy().into_owned()),
        );
        env.insert(
            "RECIPE_DIR".into(),
            Value::String(
                recipe
                    .source
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
        for (k, v) in env.clone() {
            let v = substitute(&v, &env)?;
            env.insert(k, v);
        }
        reject_backend(&env)?;
        cache::initialize(&mut env, &recipe.identifier)?;
        let manifests = autopkg_processors::contract();
        let mut input = env.clone();
        input.remove("GITHUB_TOKEN");
        receipt = vec![Value::Dictionary(Dictionary::from_iter([(
            "Recipe input",
            Value::Dictionary(input),
        )]))];
        let verbose = env
            .get("verbose")
            .and_then(Value::as_unsigned_integer)
            .unwrap_or(0);
        if verbose > 2 {
            autopkg_platform::text_println!("{}", pretty_repr(&Value::Dictionary(env.clone()), 0));
        }
        for step in &recipe.process {
            if verbose > 0 {
                autopkg_platform::text_println!("{}", step.processor);
            }
            for (k, v) in &step.arguments {
                let v = substitute(v, &env)?;
                env.insert(k.clone(), v);
            }
            reject_backend(&env)?;
            let manifest = &manifests["processors"][&step.processor];
            let inputs = manifest["input_variables"]
                .as_object()
                .map(|vars| {
                    vars.keys()
                        .filter_map(|key| env.get(key).map(|v| (key.clone(), v.clone())))
                        .collect::<Dictionary>()
                })
                .unwrap_or_default();
            if verbose > 1 {
                autopkg_platform::text_println!(
                    "{}",
                    pretty_repr(
                        &Value::Dictionary(Dictionary::from_iter([(
                            "Input",
                            Value::Dictionary(inputs.clone())
                        )])),
                        0
                    )
                );
            }
            let execution = if let Some(preferences) = &options.preferences {
                autopkg_processors::execute_with_outputs_and_preferences(
                    &step.processor,
                    &mut env,
                    preferences,
                )
            } else {
                autopkg_processors::execute_with_outputs(&step.processor, &mut env)
            };
            let output_names = execution.map_err(|e| {
                autopkg_platform::text_eprintln!("{e}");
                format!(
                    "Error in {}: Processor: {}: Error: {e}",
                    recipe.identifier, step.processor
                )
            })?;
            let outputs = output_names
                .into_iter()
                .filter_map(|key| {
                    env.get(&key)
                        .filter(|v| truthy(v))
                        .map(|v| (key, v.clone()))
                })
                .collect::<Dictionary>();
            if verbose > 1 {
                autopkg_platform::text_println!(
                    "{}",
                    pretty_repr(
                        &Value::Dictionary(Dictionary::from_iter([(
                            "Output",
                            Value::Dictionary(outputs.clone())
                        )])),
                        0
                    )
                );
            }
            receipt.push(Value::Dictionary(Dictionary::from_iter([
                ("Processor", Value::String(step.processor.clone())),
                ("Input", Value::Dictionary(inputs)),
                ("Output", Value::Dictionary(outputs)),
            ])));
            executed.push(step.processor.clone());
            if env.get("stop_processing_recipe").is_some_and(truthy) {
                stopped = true;
                break;
            }
        }
        if verbose > 2 {
            autopkg_platform::text_println!("{}", pretty_repr(&Value::Dictionary(env.clone()), 0));
        }
        Ok(())
    })();
    let mut result = RunResult {
        environment: env,
        executed,
        stopped,
        receipt,
    };
    match outcome {
        Ok(()) => Ok(result),
        Err(message) => {
            result
                .receipt
                .push(Value::Dictionary(Dictionary::from_iter([(
                    "RecipeError",
                    Value::String(message.trim_end().to_owned()),
                )])));
            Err(RunFailure {
                message,
                partial: Box::new(result),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_names_resolve_one_folder_down_in_folder_order() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for dir in [first.join("Vendor"), second.clone()] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let recipe = "Identifier: org.app\nInput: {}\nProcess: []\n";
        let nested = first.join("Vendor/App.download.recipe.yaml");
        std::fs::write(&nested, recipe).unwrap();
        std::fs::write(second.join("App.download.recipe.yaml"), recipe).unwrap();
        let dirs = [first.clone(), second.clone()];
        // An earlier folder's subfolder outranks a later folder's top level.
        assert_eq!(locate("App.download", &dirs).unwrap(), nested);
        assert_eq!(locate("App.download.recipe.yaml", &dirs).unwrap(), nested);
        // A folder's top level outranks its own subfolders.
        let top = first.join("App.download.recipe");
        std::fs::write(&top, recipe).unwrap();
        assert_eq!(locate("App.download", &dirs).unwrap(), top);
        // Names never match extensionless files below the top level.
        std::fs::write(first.join("Vendor/Tool"), recipe).unwrap();
        assert!(locate("Tool", &[first]).is_err());
    }
    #[test]
    fn identifier_scan_is_two_levels_deep_and_skips_unreadable_folders() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("Vendor/Deeper")).unwrap();
        std::fs::create_dir_all(root.join("Locked")).unwrap();
        let write = |path: PathBuf, identifier: &str| {
            std::fs::write(
                path,
                format!("Identifier: {identifier}\nInput: {{}}\nProcess: []\n"),
            )
            .unwrap()
        };
        write(root.join("Vendor/App.recipe.yaml"), "org.shallow");
        write(root.join("Vendor/Deeper/App.recipe.yaml"), "org.deep");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("Locked"), std::fs::Permissions::from_mode(0o000))
                .unwrap();
        }
        let dirs = [root.to_owned()];
        let shallow = locate("org.shallow", &dirs);
        let deep = locate("org.deep", &dirs);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("Locked"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert_eq!(shallow.unwrap(), root.join("Vendor/App.recipe.yaml"));
        assert_eq!(deep.unwrap_err(), "Could not find parent recipe org.deep");
    }
    #[cfg(unix)]
    #[test]
    fn short_name_lookup_ignores_symlinked_subfolders_and_files() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(outside.join("Vendor")).unwrap();
        std::fs::create_dir_all(repo.join("Real")).unwrap();
        let recipe = "Identifier: org.app\nInput: {}\nProcess: []\n";
        std::fs::write(outside.join("Vendor/App.recipe.yaml"), recipe).unwrap();
        std::fs::write(outside.join("Other.recipe.yaml"), recipe).unwrap();
        std::os::unix::fs::symlink(outside.join("Vendor"), repo.join("Linked")).unwrap();
        std::os::unix::fs::symlink(
            outside.join("Other.recipe.yaml"),
            repo.join("Real/Other.recipe.yaml"),
        )
        .unwrap();
        for name in ["App", "Other"] {
            assert_eq!(
                locate(name, std::slice::from_ref(&repo)).unwrap_err(),
                format!("Could not find parent recipe {name}")
            );
        }
    }
    #[cfg(windows)]
    #[test]
    fn recipe_paths_remove_only_verbatim_windows_prefixes() {
        assert_eq!(
            normal_recipe_path(PathBuf::from(r"\\?\C:\recipes\App.recipe")),
            PathBuf::from(r"C:\recipes\App.recipe")
        );
        assert_eq!(
            normal_recipe_path(PathBuf::from(r"\\?\UNC\server\share\App.recipe")),
            PathBuf::from(r"\\server\share\App.recipe")
        );
    }
    #[test]
    fn receipts_keep_dynamic_outputs_without_leaking_unrelated_environment() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let temp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "HTTP fixture was not requested"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert_ne!(count, 0, "HTTP fixture received an incomplete request");
                request.extend_from_slice(&buffer[..count]);
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nversion=1.2.3").unwrap();
        });
        let recipe = Recipe {
            identifier: "org.dynamic".into(),
            source: temp.path().join("dynamic.recipe"),
            input: Dictionary::from_iter([
                ("url", Value::from(format!("http://{address}/version"))),
                ("unrelated", Value::from("do not report")),
                ("output_string", Value::from("previous declared value")),
            ]),
            process: vec![
                Step {
                    processor: "URLTextSearcher".into(),
                    arguments: Dictionary::from_iter([
                        (
                            "re_pattern",
                            Value::from("version=(?P<version>[0-9.]+)(?P<absent>x)?"),
                        ),
                        ("result_output_var_name", Value::from("release")),
                    ]),
                },
                Step {
                    processor: "FindAndReplace".into(),
                    arguments: Dictionary::from_iter([
                        ("input_string", Value::from("%release%")),
                        ("find", Value::from(".")),
                        ("replace", Value::from("-")),
                        ("result_output_var_name", Value::from("custom_version")),
                    ]),
                },
                Step {
                    processor: "VariableSetter".into(),
                    arguments: Dictionary::from_iter([(
                        "arbitrary",
                        Value::from("not a declared output"),
                    )]),
                },
            ],
        };
        let result = run_recipe(&recipe, Dictionary::new(), &RunOptions::default()).unwrap();
        server.join().unwrap();
        let output = |index: usize| {
            result.receipt[index].as_dictionary().unwrap()["Output"]
                .as_dictionary()
                .unwrap()
        };
        assert_eq!(
            output(1),
            &Dictionary::from_iter([
                ("release", Value::from("1.2.3")),
                ("version", Value::from("1.2.3")),
            ])
        );
        assert!(result.environment["absent"].is_null());
        assert_eq!(
            output(2),
            &Dictionary::from_iter([
                ("custom_version", Value::from("1-2-3")),
                ("output_string", Value::from("previous declared value")),
            ])
        );
        assert!(output(3).is_empty());
    }
    #[test]
    fn dynamic_deprecation_output_reaches_receipt_and_summary_once() {
        let temp = tempfile::tempdir().unwrap();
        let recipe = Recipe {
            identifier: "org.deprecated".into(),
            source: temp.path().join("Legacy.recipe.plist"),
            input: Dictionary::new(),
            process: vec![
                Step {
                    processor: "MunkiCatalogBuilder".into(),
                    arguments: Dictionary::new(),
                },
                Step {
                    processor: "VariableSetter".into(),
                    arguments: Dictionary::new(),
                },
            ],
        };
        let result = run_recipe(&recipe, Dictionary::new(), &RunOptions::default()).unwrap();
        let mut report = report::Report::default();
        report.add_receipt(&result.receipt).unwrap();
        let summary = report.summary_results["deprecation_summary_result"]
            .as_dictionary()
            .unwrap();
        let rows = summary["data_rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].as_dictionary().unwrap()["name"].as_string(),
            Some("Legacy")
        );
        assert_eq!(rows[0].as_dictionary().unwrap()["warning"].as_string(), Some("MunkiCatalogBuilder was deprecated in AutoPkg version 2.7.5 and may be removed in a future release."));
        let path = temp.path().join("report.plist");
        report.write(&path).unwrap();
        assert_eq!(Value::from_file(path).unwrap(), report.to_plist());
    }
    #[cfg(unix)]
    #[test]
    fn active_office_does_not_repeat_an_inherited_deprecation_summary() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let feed = temp.path().join("office.plist");
        let mut metadata = Dictionary::from_iter([
            (
                "Location",
                Value::from("https://example.invalid/App_Updater.pkg"),
            ),
            ("Title", Value::from("Office fixture")),
            ("Update Version", Value::from("16.99")),
            ("Minimum OS", Value::from("12.0")),
        ]);
        metadata.insert(
            "Trigger Condition".into(),
            Value::Array(vec!["and".into(), "Registered File".into()]),
        );
        Value::Array(vec![metadata.into()])
            .to_file_xml(&feed)
            .unwrap();
        let curl = temp.path().join("curl-fixture");
        // Only the HTTP transport is replaced; normal processor and report paths run.
        std::fs::write(
            &curl,
            "#!/bin/sh\n/bin/cat \"$(dirname \"$0\")/office.plist\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
        for product in ["Word2019", "RemoteDesktop"] {
            let recipe = Recipe {
                identifier: format!("org.activeoffice.{product}"),
                source: temp.path().join("Office.recipe"),
                input: Dictionary::from_iter([
                    (
                        "CURL_PATH",
                        Value::from(curl.to_string_lossy().into_owned()),
                    ),
                    (
                        "CACHE_DIR",
                        Value::from(temp.path().join("cache").to_string_lossy().into_owned()),
                    ),
                ]),
                process: vec![
                    Step {
                        processor: "DeprecationWarning".into(),
                        arguments: Dictionary::from_iter([(
                            "warning_message",
                            Value::from("prior warning"),
                        )]),
                    },
                    Step {
                        processor: "MSOfficeMacURLandUpdateInfoProvider".into(),
                        arguments: Dictionary::from_iter([("product", Value::from(product))]),
                    },
                ],
            };
            let result = run_recipe(&recipe, Dictionary::new(), &RunOptions::default()).unwrap();
            assert_eq!(result.executed.len(), 2);
            let output = result.receipt[2].as_dictionary().unwrap()["Output"]
                .as_dictionary()
                .unwrap();
            assert_eq!(output["version"].as_string(), Some("16.99"));
            assert!(!output.contains_key("deprecation_summary_result"));
            assert!(result
                .environment
                .contains_key("deprecation_summary_result"));
            let mut report = report::Report::default();
            report.add_receipt(&result.receipt).unwrap();
            let summary = report.summary_results["deprecation_summary_result"]
                .as_dictionary()
                .unwrap();
            let rows = summary["data_rows"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(
                rows[0].as_dictionary().unwrap()["warning"].as_string(),
                Some("prior warning")
            );
        }
    }
    #[test]
    fn office_deprecation_stops_recipe_and_reaches_report() {
        let temp = tempfile::tempdir().unwrap();
        let recipe = Recipe {
            identifier: "org.office2016".into(),
            source: temp.path().join("Office2016.recipe"),
            input: Dictionary::new(),
            process: vec![
                Step {
                    processor: "MSOfficeMacURLandUpdateInfoProvider".into(),
                    arguments: Dictionary::from_iter([("product", Value::from("Word2016"))]),
                },
                Step {
                    processor: "FileCreator".into(),
                    arguments: Dictionary::from_iter([
                        (
                            "file_path",
                            Value::from(
                                temp.path()
                                    .join("must-not-exist")
                                    .to_string_lossy()
                                    .into_owned(),
                            ),
                        ),
                        ("file_content", Value::from("stopped")),
                    ]),
                },
            ],
        };
        let result = run_recipe(&recipe, Dictionary::new(), &RunOptions::default()).unwrap();
        assert_eq!(result.executed.len(), 1);
        assert!(result.stopped);
        assert_eq!(result.receipt.len(), 2);
        assert!(!temp.path().join("must-not-exist").exists());
        let mut report = report::Report::default();
        report.add_receipt(&result.receipt).unwrap();
        let summary = report.summary_results["deprecation_summary_result"]
            .as_dictionary()
            .unwrap();
        let rows = summary["data_rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].as_dictionary().unwrap()["warning"]
            .as_string()
            .unwrap()
            .contains("Word2016"));
    }
    #[test]
    fn null_inputs_substitute_like_python_none_and_stay_typed() {
        let env = Dictionary::from_iter([
            ("nothing", Value::Null),
            ("container", Value::Array(vec![Value::Null])),
        ]);
        assert!(!truthy(&Value::Null));
        assert_eq!(
            substitute(&Value::String("before%nothing%after".into()), &env)
                .unwrap()
                .as_string(),
            Some("beforeafter")
        );
        assert_eq!(
            substitute(&Value::String("%container%".into()), &env)
                .unwrap()
                .as_string(),
            Some("[None]")
        );
        assert_eq!(
            substitute(&Value::Array(vec![Value::Null]), &env).unwrap(),
            Value::Array(vec![Value::Null])
        );
    }

    #[test]
    fn cache_preflight_precedes_directory_creation() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let mut recipe = Recipe {
            identifier: "org.test".into(),
            source: PathBuf::from("test.recipe"),
            input: Dictionary::new(),
            process: vec![Step {
                processor: "CustomProcessor".into(),
                arguments: Dictionary::new(),
            }],
        };
        let initial = Dictionary::from_iter([(
            "CACHE_DIR",
            Value::String(cache.to_string_lossy().into_owned()),
        )]);
        assert!(run_recipe(&recipe, initial.clone(), &RunOptions::default()).is_err());
        assert!(!cache.exists());
        recipe.process.clear();
        recipe.identifier = "../escape".into();
        assert!(run_recipe(&recipe, initial.clone(), &RunOptions::default())
            .unwrap_err()
            .contains("outside CACHE_DIR"));
        assert!(!cache.exists());
        recipe.identifier = "org.test".into();
        let result = run_recipe(&recipe, initial, &RunOptions::default()).unwrap();
        assert!(cache.join("org.test").is_dir());
        assert_eq!(
            result.environment["RECIPE_CACHE_DIR"].as_string(),
            cache.join("org.test").to_str()
        );
    }
    #[test]
    fn processor_failure_preserves_completed_receipt_and_environment() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let recipe = Recipe {
            identifier: "org.test".into(),
            source: PathBuf::from("test.recipe"),
            input: Dictionary::new(),
            process: vec![
                Step {
                    processor: "VariableSetter".into(),
                    arguments: Dictionary::from_iter([(
                        "marker",
                        Value::String("retained".into()),
                    )]),
                },
                Step {
                    processor: "FileMover".into(),
                    arguments: Dictionary::from_iter([
                        (
                            "source",
                            Value::String(missing.to_string_lossy().into_owned()),
                        ),
                        (
                            "target",
                            Value::String(
                                temp.path().join("target").to_string_lossy().into_owned(),
                            ),
                        ),
                    ]),
                },
            ],
        };
        let initial = Dictionary::from_iter([("GITHUB_TOKEN", Value::String("secret".into()))]);
        let failure = run_recipe_detailed(&recipe, initial, &RunOptions::default()).unwrap_err();
        assert_eq!(failure.partial.executed, vec!["VariableSetter"]);
        assert_eq!(
            failure.partial.environment["marker"].as_string(),
            Some("retained")
        );
        assert_eq!(failure.partial.receipt.len(), 3);
        assert!(
            !failure.partial.receipt[0].as_dictionary().unwrap()["Recipe input"]
                .as_dictionary()
                .unwrap()
                .contains_key("GITHUB_TOKEN")
        );
        assert!(failure.partial.receipt[2]
            .as_dictionary()
            .unwrap()
            .contains_key("RecipeError"));
    }
    fn write(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
    }
    #[test]
    fn inherits_inputs_and_appends_steps() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("parent.recipe.yaml"),"Identifier: org.parent\nInput:\n  NAME: parent\n  kept: true\nProcess:\n- Processor: EndOfCheckPhase\n");
        let child = dir.path().join("child.recipe.yaml");
        write(&child,"Identifier: org.child\nParentRecipe: org.parent\nInput:\n  NAME: child\nProcess:\n- Processor: VariableSetter\n");
        let recipe = load_recipe(&child, &[]).unwrap();
        assert_eq!(recipe.identifier, "org.child");
        assert_eq!(recipe.input["NAME"].as_string(), Some("child"));
        assert_eq!(recipe.input["kept"].as_boolean(), Some(true));
        assert_eq!(
            recipe
                .process
                .iter()
                .map(|s| s.processor.as_str())
                .collect::<Vec<_>>(),
            vec!["EndOfCheckPhase", "VariableSetter"]
        );
    }
    #[test]
    fn substitution_is_recursive_single_pass_and_preserves_missing_strings() {
        let env = Dictionary::from_iter([
            ("name", Value::String("%other%".into())),
            ("disabled", Value::Boolean(false)),
        ]);
        assert_eq!(
            substitute(&Value::String("%name% %disabled%".into()), &env).unwrap(),
            Value::String("%other% ".into())
        );
        assert_eq!(
            substitute(&Value::String("%name% %missing%".into()), &env).unwrap(),
            Value::String("%name% %missing%".into())
        );
        let nested = Value::Array(vec![
            Value::Integer(12.into()),
            Value::String("%name%".into()),
        ]);
        assert_eq!(
            substitute(&nested, &env).unwrap(),
            Value::Array(vec![
                Value::Integer(12.into()),
                Value::String("%other%".into())
            ])
        );
    }
    #[test]
    fn substitution_renders_python_containers_and_binary_data() {
        let values = Value::Array(vec![
            false.into(),
            "a'b".into(),
            Value::Real(1.0),
            Value::Data(vec![0, 255]),
        ]);
        assert_eq!(
            python_string(&values),
            "[False, \"a'b\", 1.0, b'\\x00\\xff']"
        );
        assert_eq!(python_string(&Value::Real(1e-7)), "1e-07");
    }
    #[test]
    fn late_cache_inputs_retain_python_single_pass_substitution() {
        // RelocatablePython.build has this Input -> Arguments indirection.
        // Python substitutes Input before process() defines RECIPE_CACHE_DIR;
        // injecting the argument does not recursively expand replacement text.
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("late-cache.recipe.yaml");
        write(&source, "Identifier: org.late-cache\nInput:\n  CACHE_DIR: ignored-recipe-cache\n  REQUIREMENTS_PATH: '%RECIPE_CACHE_DIR%/relocatable-python/requirements_python3_recommended.txt'\nProcess:\n- Processor: VariableSetter\n  Arguments:\n    requirements_path: '%REQUIREMENTS_PATH%'\n    direct_cache: '%RECIPE_CACHE_DIR%/direct'\n");
        let recipe = load_recipe(&source, &[]).unwrap();
        let cli_cache = dir.path().join("cli-cache");
        let result = run_recipe(
            &recipe,
            Dictionary::from_iter([("CACHE_DIR", cli_cache.to_string_lossy().into_owned())]),
            &RunOptions::default(),
        )
        .unwrap();
        let cache = cli_cache.join("org.late-cache");
        assert!(cache.is_dir());
        assert_eq!(
            result.environment["CACHE_DIR"].as_string(),
            cli_cache.to_str()
        );
        assert_eq!(
            result.environment["RECIPE_CACHE_DIR"].as_string(),
            cache.to_str()
        );
        let unresolved =
            "%RECIPE_CACHE_DIR%/relocatable-python/requirements_python3_recommended.txt";
        assert_eq!(
            result.environment["REQUIREMENTS_PATH"].as_string(),
            Some(unresolved)
        );
        assert_eq!(
            result.environment["requirements_path"].as_string(),
            Some(unresolved)
        );
        // The slash is recipe text, so Windows retains it after the cache path.
        assert_eq!(
            result.environment["direct_cache"].as_string(),
            Some(format!("{}/direct", cache.display()).as_str())
        );
        let input = result.receipt[0].as_dictionary().unwrap()["Recipe input"]
            .as_dictionary()
            .unwrap();
        assert_eq!(input["REQUIREMENTS_PATH"].as_string(), Some(unresolved));
        assert_eq!(input["RECIPE_CACHE_DIR"].as_string(), cache.to_str());
    }
    #[test]
    fn rejects_custom_processor_before_file_creation() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("must-not-exist");
        let cache = dir.path().join("must-not-create-cache");
        let recipe = Recipe {
            identifier: "test".into(),
            source: dir.path().join("test.recipe"),
            input: Dictionary::new(),
            process: vec![
                Step {
                    processor: "FileCreator".into(),
                    arguments: Dictionary::from_iter([
                        (
                            "file_path",
                            Value::String(target.to_string_lossy().into_owned()),
                        ),
                        ("file_content", Value::String("test".into())),
                    ]),
                },
                Step {
                    processor: "org.custom/Processor".into(),
                    arguments: Dictionary::new(),
                },
            ],
        };
        assert!(run_recipe(
            &recipe,
            Dictionary::from_iter([("CACHE_DIR", cache.to_string_lossy().into_owned())]),
            &RunOptions::default()
        )
        .unwrap_err()
        .contains("Custom"));
        assert!(!target.exists());
        assert!(!cache.exists());
    }
    #[test]
    fn rejects_cycles_and_unverified_trust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cycle.recipe.yaml");
        write(&path, "ParentRecipe: cycle\nInput: {}\n");
        assert!(load_recipe(&path, &[]).unwrap_err().contains("cycle"));
        write(&path, "ParentRecipeTrustInfo: {}\nInput: {}\nProcess: []\n");
        assert!(load_recipe(&path, &[]).unwrap_err().contains("trust"));
    }
    #[test]
    fn rejects_python_munki_backend() {
        let d = Dictionary::from_iter([("force_munki_repo_lib", Value::Boolean(true))]);
        assert!(reject_backend(&d).is_err());
        assert!(reject_backend(&Dictionary::from_iter([(
            "MUNKI_REPO",
            Value::String("https://host/repo".into())
        )]))
        .is_err());
    }
    #[test]
    fn backend_substitution_is_rejected_before_any_steps() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("never-created");
        let recipe = Recipe {
            identifier: "test".into(),
            source: PathBuf::from("test.recipe"),
            input: Dictionary::new(),
            process: vec![
                Step {
                    processor: "FileCreator".into(),
                    arguments: Dictionary::from_iter([
                        (
                            "file_path",
                            Value::String(target.to_string_lossy().into_owned()),
                        ),
                        ("file_content", Value::String("test".into())),
                    ]),
                },
                Step {
                    processor: "VariableSetter".into(),
                    arguments: Dictionary::from_iter([(
                        "backend",
                        Value::String("PythonRepo".into()),
                    )]),
                },
                Step {
                    processor: "VariableSetter".into(),
                    arguments: Dictionary::from_iter([(
                        "MUNKI_REPO_PLUGIN",
                        Value::String("%backend%".into()),
                    )]),
                },
            ],
        };
        assert!(
            run_recipe(&recipe, Dictionary::new(), &RunOptions::default())
                .unwrap_err()
                .contains("FileRepo")
        );
        assert!(!target.exists());
    }
    #[test]
    fn accepts_registered_builtins_and_rejects_missing_inputs() {
        let mut recipe = Recipe {
            identifier: "test".into(),
            source: PathBuf::new(),
            input: Dictionary::new(),
            process: vec![Step {
                processor: "AppPkgCreator".into(),
                arguments: Dictionary::new(),
            }],
        };
        validate_recipe(&recipe, &Dictionary::new()).unwrap();
        recipe.process[0].processor = "FileCreator".into();
        assert!(validate_recipe(&recipe, &Dictionary::new())
            .unwrap_err()
            .contains("missing argument"));
    }
    #[test]
    fn binary_plist_preserves_dates_and_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("typed.recipe");
        let input = Dictionary::from_iter([
            ("blob", Value::Data(vec![0, 128, 255])),
            (
                "date",
                Value::Date(plist::Date::from_xml_format("2026-10-06T00:00:00Z").unwrap()),
            ),
        ]);
        Value::Dictionary(Dictionary::from_iter([
            ("Input", Value::Dictionary(input.clone())),
            ("Process", Value::Array(vec![])),
        ]))
        .to_file_binary(&path)
        .unwrap();
        assert_eq!(load_recipe(&path, &[]).unwrap().input, input);
    }
    #[test]
    fn cli_input_wins_and_check_includes_marker() {
        let recipe = Recipe {
            identifier: "test".into(),
            source: PathBuf::from("test.recipe"),
            input: Dictionary::from_iter([("NAME", Value::String("recipe".into()))]),
            process: vec![Step {
                processor: "EndOfCheckPhase".into(),
                arguments: Dictionary::new(),
            }],
        };
        let result = run_recipe(
            &recipe,
            Dictionary::from_iter([("NAME", Value::String("cli".into()))]),
            &RunOptions {
                check_only: true,
                ..RunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(result.environment["NAME"].as_string(), Some("cli"));
        assert_eq!(result.executed, vec!["EndOfCheckPhase"]);
    }
    #[test]
    fn check_uses_last_marker_and_requires_one() {
        let mut recipe = Recipe {
            identifier: "test".into(),
            source: PathBuf::from("test.recipe"),
            input: Dictionary::new(),
            process: vec![],
        };
        assert!(recipe.check_phase().is_err());
        for processor in [
            "EndOfCheckPhase",
            "VariableSetter",
            "EndOfCheckPhase",
            "FileCreator",
        ] {
            recipe.process.push(Step {
                processor: processor.into(),
                arguments: Dictionary::new(),
            });
        }
        let result = run_recipe(
            &recipe,
            Dictionary::new(),
            &RunOptions {
                check_only: true,
                ..RunOptions::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.executed,
            vec!["EndOfCheckPhase", "VariableSetter", "EndOfCheckPhase"]
        );
    }
}
