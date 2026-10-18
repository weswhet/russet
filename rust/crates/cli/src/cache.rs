use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

fn cache_value(value: Option<&Value>) -> Result<Option<&str>, String> {
    match value {
        None | Some(Value::Null) | Some(Value::Boolean(false)) => Ok(None),
        Some(Value::String(value)) => Ok((!value.is_empty()).then_some(value.as_str())),
        Some(Value::Integer(value)) if value.as_signed() == Some(0) => Ok(None),
        Some(Value::Real(value)) if *value == 0.0 => Ok(None),
        Some(Value::Array(value)) if value.is_empty() => Ok(None),
        Some(Value::Dictionary(value)) if value.is_empty() => Ok(None),
        Some(Value::Data(value)) if value.is_empty() => Ok(None),
        _ => Err("CACHE_DIR must be a path string".into()),
    }
}
/// Resolve the reference cache root, retaining the optional development fallback.
pub fn root(prefs: &Dictionary) -> Result<PathBuf, String> {
    root_with_override(prefs, None)
}
pub fn root_with_override(
    prefs: &Dictionary,
    override_value: Option<&Value>,
) -> Result<PathBuf, String> {
    let configured = match cache_value(override_value)? {
        Some(value) => Some(value),
        None => cache_value(prefs.get("CACHE_DIR"))?,
    };
    let path = configured
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("RUSSET_CACHE_DIR")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("~/Library/Russet/Cache"));
    autopkg_engine::cache::normalized(&path)
}

fn remove_item(path: &Path, dry_run: bool) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

fn list_tree(path: &Path, action: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Ok(());
    }
    let mut children = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    children.sort();
    for child in children {
        autopkg_platform::text_println!("{action} {}", child.display());
        list_tree(&child, action)?;
    }
    Ok(())
}

/// Clears one recipe's cache, which is named by the identifier of the recipe
/// that `run` would use, so an override's own identifier.
#[allow(clippy::too_many_arguments)]
fn clear_recipe(
    prefs: &Dictionary,
    name: &str,
    mut search: Vec<PathBuf>,
    mut overrides: Vec<PathBuf>,
    root: &Path,
    action: &str,
    verbose: usize,
    dry_run: bool,
) -> Result<i32, String> {
    let (configured_search, configured_overrides) = super::manage::recipe_directories(prefs)?;
    if search.is_empty() {
        search = configured_search;
    }
    if overrides.is_empty() {
        overrides = configured_overrides;
    }
    if !Path::new(name).is_file() {
        super::manage::ensure_recipe_map(prefs)?;
    }
    overrides.extend(search);
    let Ok(path) = super::manage::locate(name, &overrides, true) else {
        autopkg_platform::text_eprintln!("No valid recipe found for {name}");
        return Ok(1);
    };
    let raw = autopkg_engine::read_recipe(&path)?;
    let Some(identifier) = raw
        .get("Identifier")
        .or_else(|| {
            raw.get("Input")
                .and_then(Value::as_dictionary)
                .and_then(|i| i.get("IDENTIFIER"))
        })
        .and_then(Value::as_string)
    else {
        autopkg_platform::text_eprintln!("ERROR: Could not determine recipe identifier for {name}");
        return Ok(1);
    };
    let target = autopkg_engine::cache::recipe_cache_path(root, identifier)?;
    if target == root {
        return Err("Recipe cache path must be below CACHE_DIR".into());
    }
    if !target.exists() {
        autopkg_platform::text_eprintln!(
            "ERROR: Recipe cache directory does not exist: {}",
            target.display()
        );
        return Ok(1);
    }
    autopkg_platform::text_println!("{action} {}", target.display());
    if verbose > 1 {
        list_tree(&target, action)?;
    }
    if let Err(error) = remove_item(&target, dry_run) {
        autopkg_platform::text_eprintln!("ERROR: Could not remove {}: {error}", target.display());
        return Ok(1);
    }
    Ok(0)
}

pub fn run(args: &[String]) -> Result<i32, String> {
    let mut prefs = super::manage::load_preferences(None)?;
    let mut search = Vec::new();
    let mut overrides = Vec::new();
    let mut dry_run = false;
    let mut verbose = 0;
    let mut targets = Vec::new();
    let mut arguments = args.iter();
    while let Some(arg) = arguments.next() {
        match arg.as_str() {
            "--prefs" => {
                prefs = super::dictionary(arguments.next().ok_or("--prefs requires a path")?)?
            }
            "--dry-run" => dry_run = true,
            "-v" | "--verbose" => verbose += 1,
            "-vv" => verbose += 2,
            "-d" | "--search-dir" => search.push(PathBuf::from(
                arguments.next().ok_or("Search directory requires a path")?,
            )),
            "--override-dir" => overrides.push(PathBuf::from(
                arguments
                    .next()
                    .ok_or("Override directory requires a path")?,
            )),
            flag if flag.starts_with('-') => {
                return Err(format!("Unknown clear-cache option '{flag}'"))
            }
            _ => targets.push(arg),
        }
    }
    if targets.len() != 1 {
        return super::options::usage_failure("clear-cache", None, 1);
    }
    let root = root(&prefs)?;
    let root = root
        .canonicalize()
        .map_err(|e| format!("Cache directory does not exist: {}: {e}", root.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "Cache directory does not exist: {}",
            root.display()
        ));
    }
    let action = if dry_run { "Would remove" } else { "Removing" };
    let items = if targets[0] == "all" {
        let mut entries = std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort();
        if verbose == 0 && !entries.is_empty() {
            autopkg_platform::text_println!("{action} all cached items from {}", root.display());
        }
        entries
    } else {
        return clear_recipe(
            &prefs, targets[0], search, overrides, &root, action, verbose, dry_run,
        );
    };
    let mut success = true;
    for item in items {
        if verbose > 0 {
            autopkg_platform::text_println!("{action} {}", item.display());
        }
        if verbose > 1 {
            list_tree(&item, action)?;
        }
        if let Err(error) = remove_item(&item, dry_run) {
            autopkg_platform::text_eprintln!("ERROR: Could not remove {}: {error}", item.display());
            success = false;
        }
    }
    Ok(if success { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_cache_precedence_and_null_empty_fallback_are_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let preference = temp.path().join("preferences");
        let explicit = temp.path().join("explicit");
        let prefs = Dictionary::from_iter([(
            "CACHE_DIR",
            Value::from(preference.to_string_lossy().into_owned()),
        )]);
        assert_eq!(
            root_with_override(&prefs, Some(&Value::Null)).unwrap(),
            preference
        );
        assert_eq!(
            root_with_override(&prefs, Some(&Value::from(""))).unwrap(),
            preference
        );
        assert_eq!(
            root_with_override(
                &prefs,
                Some(&Value::from(explicit.to_string_lossy().into_owned()))
            )
            .unwrap(),
            explicit
        );
        let invalid_prefs = Dictionary::from_iter([("CACHE_DIR", Value::Boolean(true))]);
        assert_eq!(
            root_with_override(
                &invalid_prefs,
                Some(&Value::from(explicit.to_string_lossy().into_owned()))
            )
            .unwrap(),
            explicit
        );
        assert!(!preference.exists());
        assert!(!explicit.exists());
        assert!(cache_value(Some(&Value::Null)).unwrap().is_none());
        assert!(cache_value(Some(&Value::from(""))).unwrap().is_none());
        assert!(cache_value(Some(&Value::Boolean(false))).unwrap().is_none());
        assert!(cache_value(Some(&Value::Boolean(true))).is_err());
    }
    #[test]
    fn recipe_cache_uses_the_override_identifier() {
        let temp = tempfile::tempdir().unwrap();
        let (recipes, overrides, cache) = (
            temp.path().join("recipes"),
            temp.path().join("overrides"),
            temp.path().join("cache"),
        );
        for dir in [&recipes, &overrides, &cache.join("org.test.app")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let recipe = |path: PathBuf, identifier: &str| {
            Value::Dictionary(Dictionary::from_iter([
                ("Identifier", Value::from(identifier)),
                ("Input", Value::Dictionary(Dictionary::new())),
                ("Process", Value::Array(vec![])),
            ]))
            .to_file_xml(path)
            .unwrap()
        };
        recipe(recipes.join("App.recipe"), "org.test.app");
        let text = |p: &Path| Value::from(p.to_string_lossy().into_owned());
        let prefs = Dictionary::from_iter([
            ("DISABLE_RECIPE_MAP", Value::Boolean(true)),
            ("RECIPE_SEARCH_DIRS", Value::Array(vec![text(&recipes)])),
            ("RECIPE_OVERRIDE_DIRS", Value::Array(vec![text(&overrides)])),
        ]);
        let clear = |overrides: Vec<PathBuf>| {
            clear_recipe(
                &prefs,
                "App",
                vec![],
                overrides,
                &cache,
                "Removing",
                0,
                true,
            )
        };
        assert_eq!(clear(vec![]).unwrap(), 0);
        // The override's cache folder doesn't exist, even though its
        // parent's does.
        recipe(overrides.join("App.recipe"), "local.App");
        assert_eq!(clear(vec![]).unwrap(), 1);
        assert_eq!(clear(vec![temp.path().join("none")]).unwrap(), 0);
        assert!(cache.join("org.test.app").is_dir());
    }
    #[test]
    fn dry_run_preserves_contents() {
        let temp = tempfile::tempdir().unwrap();
        let item = temp.path().join("cached");
        std::fs::write(&item, "data").unwrap();
        remove_item(&item, true).unwrap();
        assert!(item.exists());
        remove_item(&item, false).unwrap();
        assert!(!item.exists());
        assert!(temp.path().exists());
    }
    #[cfg(unix)]
    #[test]
    fn deleting_symlink_preserves_external_files() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("keep");
        std::fs::write(&file, "data").unwrap();
        let link = temp.path().join("alias");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        remove_item(&link, false).unwrap();
        assert!(file.exists());
        assert!(!link.exists());
    }
}
