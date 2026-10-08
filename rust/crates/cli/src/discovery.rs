//! Read-only recipe discovery and inspection.
use autopkg_engine::{preferences::Preferences, read_recipe};
use plist::{Dictionary, Value};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

const EXTENSIONS: [&str; 3] = [".recipe", ".recipe.plist", ".recipe.yaml"];
#[derive(Default)]
struct Options {
    search: Vec<PathBuf>,
    overrides: Vec<PathBuf>,
    prefs: Dictionary,
    identifiers: bool,
    paths: bool,
    plist: bool,
    all: bool,
    recipes: Vec<String>,
    help: bool,
}
fn expand(path: &str) -> PathBuf {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(""));
        }
    }
    PathBuf::from(path)
}
fn configured(prefs: &Dictionary, key: &str) -> Result<Vec<PathBuf>, String> {
    match prefs.get(key) {
        None => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![expand(s)]),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| {
                v.as_string()
                    .map(expand)
                    .ok_or_else(|| format!("{key} must contain paths"))
            })
            .collect(),
        _ => Err(format!("{key} must be a string or array")),
    }
}
fn parse(verb: &str, args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        prefs: crate::manage::load_preferences(None)?,
        ..Options::default()
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--prefs" => {
                o.prefs.extend(
                    Preferences::load(&expand(iter.next().ok_or("--prefs requires a path")?))?
                        .values,
                );
            }
            "-d" | "--search-dir" => o
                .search
                .push(expand(iter.next().ok_or("--search-dir requires a path")?)),
            "--override-dir" => o
                .overrides
                .push(expand(iter.next().ok_or("--override-dir requires a path")?)),
            "-h" | "--help" => o.help = true,
            "-i" | "--with-identifiers" if verb == "list-recipes" => o.identifiers = true,
            "-p" | "--with-paths" if verb == "list-recipes" => o.paths = true,
            "--plist" if verb == "list-recipes" => o.plist = true,
            "-a" | "--show-all" if verb == "list-recipes" => o.all = true,
            "-q" | "--quiet" if verb == "info" => {}
            "--" => {
                o.recipes.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unsupported {verb} option '{flag}'"))
            }
            _ => o.recipes.push(arg.clone()),
        }
    }
    if o.plist && (o.identifiers || o.paths) {
        return Err(
            "It is invalid to specify '--with-identifiers' or '--with-paths' with '--plist'."
                .into(),
        );
    }
    if o.all && !(o.identifiers || o.paths || o.plist) {
        return Err("The '--show-all' option is only valid when used with '--with-paths', '--with-identifiers', or '--plist' options.".into());
    }
    if o.search.is_empty() {
        o.search = configured(&o.prefs, "RECIPE_SEARCH_DIRS")?;
    }
    if o.overrides.is_empty() {
        o.overrides = configured(&o.prefs, "RECIPE_OVERRIDE_DIRS")?;
    }
    if o.search.is_empty() {
        o.search = [".", "~/Library/AutoPkg/Recipes", "/Library/AutoPkg/Recipes"]
            .iter()
            .map(|s| expand(s))
            .collect();
    }
    if o.overrides.is_empty() {
        o.overrides
            .push(expand("~/Library/AutoPkg/RecipeOverrides"));
    }
    Ok(o)
}
fn shortname(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    EXTENSIONS
        .iter()
        .find_map(|ext| name.strip_suffix(ext).map(str::to_owned))
}
fn files(directory: &Path, nested: bool) -> Result<Vec<PathBuf>, String> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let absolute = if directory.is_absolute() {
        directory.to_owned()
    } else {
        env::current_dir()
            .map_err(|e| e.to_string())?
            .join(directory)
    };
    let mut result = Vec::new();
    for entry in fs::read_dir(&absolute).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.starts_with('.'))
        {
            continue;
        }
        if path.is_dir() && nested {
            result.extend(files(&path, false)?);
        } else if path.is_file() && shortname(&path).is_some() {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}
fn valid(d: &Dictionary, is_override: bool) -> bool {
    d.contains_key("Input")
        && (d.contains_key("Recipe")
            || d.contains_key("ParentRecipe")
            || (!is_override && d.contains_key("Process")))
}
fn identifier(d: &Dictionary) -> &str {
    d.get("Identifier")
        .and_then(Value::as_string)
        .or_else(|| {
            d.get("Input")
                .and_then(Value::as_dictionary)?
                .get("IDENTIFIER")?
                .as_string()
        })
        .unwrap_or("")
}
fn collect(o: &Options) -> Result<Vec<Dictionary>, String> {
    let mut result: Vec<Dictionary> = Vec::new();
    for (directories, is_override) in [(&o.search, false), (&o.overrides, true)] {
        for directory in directories {
            for path in files(directory, !is_override)? {
                let Ok(mut recipe) = read_recipe(&path) else {
                    continue;
                };
                if !valid(&recipe, is_override) {
                    continue;
                }
                let name = shortname(&path).unwrap();
                if !is_override && !recipe.contains_key("Identifier") {
                    let id = identifier(&recipe).to_owned();
                    if !id.is_empty() {
                        recipe.insert("Identifier".into(), id.into());
                    }
                }
                if is_override {
                    if (o.identifiers || o.paths || o.plist) && !o.all {
                        result.retain(|r| {
                            !(r.get("Name").and_then(Value::as_string) == Some(&name)
                                && r.get("Identifier") == recipe.get("ParentRecipe"))
                        });
                    }
                    recipe.insert("IsOverride".into(), true.into());
                }
                recipe.insert("Name".into(), name.into());
                recipe.insert("Path".into(), path.to_string_lossy().into_owned().into());
                result.push(recipe);
            }
        }
    }
    result.sort_by_key(|r| r["Name"].as_string().unwrap().to_lowercase());
    Ok(result)
}
fn text_list(rows: &[Dictionary], o: &Options) -> String {
    let augmented = o.identifiers || o.paths;
    let name_width = rows
        .iter()
        .map(|r| r["Name"].as_string().unwrap().chars().count())
        .max()
        .unwrap_or(0)
        + 1;
    let id_width = if o.identifiers {
        rows.iter()
            .map(|r| {
                r.get("Identifier")
                    .and_then(Value::as_string)
                    .unwrap_or("")
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(0)
            + 1
    } else {
        1
    };
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for row in rows {
        let name = row["Name"].as_string().unwrap();
        let id = if o.identifiers {
            row.get("Identifier")
                .and_then(Value::as_string)
                .unwrap_or("")
        } else {
            ""
        };
        let mut path = if o.paths {
            row["Path"].as_string().unwrap().to_owned()
        } else {
            String::new()
        };
        if let Ok(home) = env::var("HOME") {
            if !home.is_empty() {
                path = path.replace(&home, "~");
            }
        }
        let line = if augmented {
            format!("{name:<name_width$} {id:<id_width$} {path:<20}")
        } else {
            name.to_owned()
        };
        if seen.insert(line.clone()) {
            output.push(line);
        }
    }
    output.join("\n")
}
fn pretty_repr(value: &Value, indent: usize) -> String {
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
fn sorted(value: &mut Value) {
    match value {
        Value::Dictionary(d) => {
            d.sort_keys();
            for (_, v) in d.iter_mut() {
                sorted(v);
            }
        }
        Value::Array(a) => {
            for v in a {
                sorted(v)
            }
        }
        _ => {}
    }
}
fn python_repr(v: &Value) -> String {
    match v {
        Value::String(s) => format!(
            "'{}'",
            s.replace('\\', "\\\\")
                .replace('\'', "\\'")
                .replace('\n', "\\n")
        ),
        Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
        Value::Integer(i) => i.to_string(),
        Value::Real(f) => f.to_string(),
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(python_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Dictionary(d) => {
            let mut entries: Vec<_> = d.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(k, v)| format!(
                        "{}: {}",
                        python_repr(&Value::String((*k).clone())),
                        python_repr(v)
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        other => format!("{other:?}"),
    }
}
fn summary(
    path: &Path,
    dirs: &[PathBuf],
    active: &mut HashSet<PathBuf>,
) -> Result<Dictionary, String> {
    let path = autopkg_engine::normal_recipe_path(path.canonicalize().map_err(|e| e.to_string())?);
    if !active.insert(path.clone()) {
        return Err(format!("Recipe inheritance cycle at {}", path.display()));
    }
    let child = read_recipe(&path)?;
    let input = child
        .get("Input")
        .and_then(Value::as_dictionary)
        .ok_or("Recipe requires an Input dictionary")?;
    let mut result = if let Some(parent) = child.get("ParentRecipe").or_else(|| child.get("Recipe"))
    {
        let parent = parent.as_string().ok_or("ParentRecipe must be a string")?;
        let mut search = dirs.to_vec();
        search.push(path.parent().unwrap().to_owned());
        let mut found = None;
        for dir in &search {
            for candidate in files(dir, true)? {
                let matches = shortname(&candidate).as_deref() == Some(parent)
                    || candidate.file_name().and_then(|n| n.to_str()) == Some(parent)
                    || read_recipe(&candidate)
                        .map(|d| identifier(&d) == parent)
                        .unwrap_or(false);
                if matches {
                    found = Some(candidate);
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let parent_path = found.ok_or_else(|| format!("No valid recipe found for {parent}"))?;
        let mut result = summary(&parent_path, &search, active)?;
        let mut parents = vec![result["RECIPE_PATH"].clone()];
        if let Some(prior) = result.get("PARENT_RECIPES").and_then(Value::as_array) {
            parents.extend(prior.clone());
        }
        result.insert("PARENT_RECIPES".into(), parents.into());
        result
    } else {
        let mut result = Dictionary::new();
        result.insert("Input".into(), Dictionary::new().into());
        result.insert("Process".into(), Vec::<Value>::new().into());
        result
    };
    result
        .get_mut("Input")
        .unwrap()
        .as_dictionary_mut()
        .unwrap()
        .extend(input.clone());
    if let Some(process) = child.get("Process") {
        result
            .get_mut("Process")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .extend(
                process
                    .as_array()
                    .ok_or("Process must be an array")?
                    .clone(),
            );
    }
    if let Some(description) = child.get("Description") {
        result.insert("Description".into(), description.clone());
    }
    result.insert("Identifier".into(), identifier(&child).into());
    result.insert(
        "RECIPE_PATH".into(),
        path.to_string_lossy().into_owned().into(),
    );
    active.remove(&path);
    Ok(result)
}
fn info(o: &Options) -> Result<(), String> {
    if o.recipes.is_empty() {
        autopkg_platform::text_println!(
            "Current preferences:\n{}",
            pretty_repr(&Value::Dictionary(o.prefs.clone()), 0)
        );
        return Ok(());
    }
    if o.recipes.len() > 1 {
        return Err("Too many recipes!".into());
    }
    let name = &o.recipes[0];
    if !expand(name).is_file() {
        super::manage::ensure_recipe_map(&o.prefs)?;
    }
    let rows = collect(o)?;
    let path = if expand(name).is_file() {
        expand(name)
    } else {
        // Overrides take precedence for both matching names and identifiers.
        let matches =
            |r: &&Dictionary| r["Name"].as_string() == Some(name) || identifier(r) == name;
        let row = rows
            .iter()
            .filter(matches)
            .find(|r| r.get("IsOverride").and_then(Value::as_boolean) == Some(true))
            .or_else(|| rows.iter().find(matches))
            .ok_or_else(|| format!("No valid recipe found for {name}"))?;
        PathBuf::from(row["Path"].as_string().unwrap())
    };
    let recipe = summary(&path, &o.search, &mut HashSet::new())?;
    let has = |name: &str| {
        recipe["Process"].as_array().unwrap().iter().any(|s| {
            s.as_dictionary()
                .and_then(|s| s.get("Processor"))
                .and_then(Value::as_string)
                == Some(name)
        })
    };
    let boolean = |v| if v { "True" } else { "False" };
    autopkg_platform::text_println!(
        "Description:         {}",
        recipe
            .get("Description")
            .and_then(Value::as_string)
            .unwrap_or("")
            .lines()
            .collect::<Vec<_>>()
            .join("\n                     ")
    );
    autopkg_platform::text_println!("Identifier:          {}", identifier(&recipe));
    autopkg_platform::text_println!("Munki import recipe: {}", boolean(has("MunkiImporter")));
    autopkg_platform::text_println!("Has check phase:     {}", boolean(has("EndOfCheckPhase")));
    autopkg_platform::text_println!("Builds package:      {}", boolean(has("PkgCreator")));
    autopkg_platform::text_println!(
        "Recipe file path:    {}",
        recipe["RECIPE_PATH"].as_string().unwrap()
    );
    if let Some(parents) = recipe.get("PARENT_RECIPES").and_then(Value::as_array) {
        autopkg_platform::text_println!(
            "Parent recipe(s):    {}",
            parents
                .iter()
                .filter_map(Value::as_string)
                .collect::<Vec<_>>()
                .join("\n                     ")
        );
    }
    let input = python_repr(&recipe["Input"]);
    autopkg_platform::text_println!("Input values: \n {}", &input[1..input.len() - 1]);
    Ok(())
}
pub fn run(verb: &str, args: &[String]) -> Result<i32, String> {
    if !matches!(verb, "list-recipes" | "info") {
        return Err(format!(
            "Command '{verb}' is not implemented in this development build"
        ));
    }
    let options = parse(verb, args)?;
    if options.help {
        autopkg_platform::text_println!(
            "Usage: russet {verb} [--prefs FILE] [-d DIRECTORY] [--override-dir DIRECTORY]{}",
            if verb == "info" {
                " [-q] [RECIPE]"
            } else {
                " [-i] [-p] [-a] [--plist]"
            }
        );
        return Ok(0);
    }
    if verb == "info" {
        if options.recipes.len() > 1 {
            autopkg_platform::text_eprintln!("Too many recipes!");
            return Ok(255);
        }
        info(&options)?;
    } else {
        if !options.recipes.is_empty() {
            return Err("list-recipes does not accept recipe arguments".into());
        }
        let rows = collect(&options)?;
        if options.plist {
            let mut value = Value::Array(rows.into_iter().map(Value::Dictionary).collect());
            sorted(&mut value);
            let mut bytes = Vec::new();
            value.to_writer_xml(&mut bytes).map_err(|e| e.to_string())?;
            autopkg_platform::text_print!(
                "{}",
                String::from_utf8(bytes).map_err(|e| e.to_string())?
            );
            autopkg_platform::text_println!("\n");
        } else {
            autopkg_platform::text_println!("{}", text_list(&rows, &options));
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn write(path: &Path, id: &str, parent: Option<&str>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut d = Dictionary::new();
        d.insert("Input".into(), Dictionary::new().into());
        d.insert("Identifier".into(), id.into());
        if let Some(parent) = parent {
            d.insert("ParentRecipe".into(), parent.into());
        } else {
            d.insert("Process".into(), Vec::<Value>::new().into());
        }
        Value::Dictionary(d).to_file_xml(path).unwrap();
    }
    #[test]
    fn discovery_respects_depth_and_override_suppression() {
        let temp = tempfile::tempdir().unwrap();
        let search = temp.path().join("recipes");
        let overrides = temp.path().join("overrides");
        write(&search.join("App.download.recipe.plist"), "org.app", None);
        write(&search.join("nested/Other.recipe"), "org.other", None);
        write(
            &search.join("nested/deep/Hidden.recipe"),
            "org.hidden",
            None,
        );
        write(
            &overrides.join("App.download.recipe"),
            "local.app",
            Some("org.app"),
        );
        write(
            &overrides.join("nested/Hidden.recipe"),
            "local.hidden",
            Some("org.hidden"),
        );
        let mut o = Options {
            search: vec![search],
            overrides: vec![overrides],
            plist: true,
            ..Default::default()
        };
        let rows = collect(&o).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(identifier(&rows[0]), "local.app");
        assert_eq!(rows[0]["IsOverride"].as_boolean(), Some(true));
        o.all = true;
        assert_eq!(collect(&o).unwrap().len(), 3);
    }
    #[test]
    fn info_inherits_metadata_without_executing_or_rejecting_trust_records() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("Parent.recipe.plist");
        let child = temp.path().join("Child.recipe");
        write(&parent, "org.parent", None);
        let mut data = read_recipe(&parent).unwrap();
        data.insert("Description".into(), "Parent description".into());
        data.insert(
            "Input".into(),
            Dictionary::from_iter([("NAME", Value::String("parent".into()))]).into(),
        );
        Value::Dictionary(data).to_file_xml(&parent).unwrap();
        write(&child, "org.child", Some("org.parent"));
        let mut data = read_recipe(&child).unwrap();
        data.insert("ParentRecipeTrustInfo".into(), Dictionary::new().into());
        Value::Dictionary(data).to_file_xml(&child).unwrap();
        let merged = summary(&child, &[temp.path().to_owned()], &mut HashSet::new()).unwrap();
        assert_eq!(
            merged["Description"].as_string(),
            Some("Parent description")
        );
        assert_eq!(identifier(&merged), "org.child");
        assert_eq!(
            merged["Input"].as_dictionary().unwrap()["NAME"].as_string(),
            Some("parent")
        );
        assert_eq!(merged["PARENT_RECIPES"].as_array().unwrap().len(), 1);
        let mut data = read_recipe(&parent).unwrap();
        data.insert("ParentRecipe".into(), "org.child".into());
        Value::Dictionary(data).to_file_xml(&parent).unwrap();
        assert!(
            summary(&child, &[temp.path().to_owned()], &mut HashSet::new())
                .unwrap_err()
                .contains("cycle")
        );
    }
    #[test]
    fn display_deduplicates_plain_names_and_rejects_invalid_flags() {
        let row = Dictionary::from_iter([
            ("Name", Value::String("App".into())),
            ("Path", Value::String("/tmp/App.recipe".into())),
        ]);
        assert_eq!(text_list(&[row.clone(), row], &Options::default()), "App");
        assert!(parse("list-recipes", &["--show-all".into()]).is_err());
        assert!(parse("list-recipes", &["--plist".into(), "-i".into()]).is_err());
        assert!(parse("info", &["--pull".into()]).is_err());
    }
}
