//! `russet migrate`: copy AutoPkg's preferences into Russet's config.
//! AutoPkg's settings and folders are never changed.
use crate::config::{autopkg_layer, russet_layer};
use crate::manage::expand;
use plist::{Dictionary, Value};

/// AutoPkg's folders for these keys when they're unset. Russet's defaults
/// differ, so `migrate` writes AutoPkg's to keep finding the recipes and
/// overrides AutoPkg found.
const AUTOPKG_DEFAULT_FOLDERS: [(&str, &[&str]); 2] = [
    (
        "RECIPE_SEARCH_DIRS",
        &[".", "~/Library/AutoPkg/Recipes", "/Library/AutoPkg/Recipes"],
    ),
    (
        "RECIPE_OVERRIDE_DIRS",
        &["~/Library/AutoPkg/RecipeOverrides"],
    ),
];

/// Whether AutoPkg falls back to its default for this value, as Python's
/// `value or default` does.
fn unset(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::String(v)) => v.is_empty(),
        Some(Value::Array(v)) => v.is_empty(),
        Some(Value::Dictionary(v)) => v.is_empty(),
        Some(Value::Boolean(v)) => !v,
        Some(_) => false,
    }
}

/// AutoPkg's preferences plus its default folders for the keys that are
/// unset, when one of those folders exists. Returns the keys it added.
pub(crate) fn with_autopkg_defaults(
    values: &Dictionary,
    exists: impl Fn(&str) -> bool,
) -> (Dictionary, Vec<&'static str>) {
    let mut values = values.clone();
    let mut added = Vec::new();
    for (key, folders) in AUTOPKG_DEFAULT_FOLDERS {
        if unset(values.get(key)) && folders.iter().any(|f| *f != "." && exists(f)) {
            let folders = folders.iter().map(|f| Value::from(*f)).collect();
            values.insert(key.into(), Value::Array(folders));
            added.push(key);
        }
    }
    (values, added)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Russet had no value; AutoPkg's is copied.
    Copied,
    /// Russet already has the same value.
    Unchanged,
    /// Russet has a different value, which it keeps.
    Conflict,
    /// Russet had a different value; `--force` replaces it with AutoPkg's.
    Replaced,
}

impl Outcome {
    fn label(self) -> &'static str {
        match self {
            Self::Copied => "copied",
            Self::Unchanged => "unchanged",
            Self::Conflict => "conflict",
            Self::Replaced => "replaced",
        }
    }
    fn writes(self) -> bool {
        matches!(self, Self::Copied | Self::Replaced)
    }
}

/// What to do with each AutoPkg preference, sorted by key.
pub(crate) fn plan_migration(
    autopkg: &Dictionary,
    russet: &Dictionary,
    force: bool,
) -> Vec<(String, Outcome)> {
    let mut plan: Vec<_> = autopkg
        .iter()
        .map(|(key, value)| {
            let outcome = match russet.get(key) {
                None => Outcome::Copied,
                Some(existing) if existing == value => Outcome::Unchanged,
                Some(_) if force => Outcome::Replaced,
                Some(_) => Outcome::Conflict,
            };
            (key.clone(), outcome)
        })
        .collect();
    plan.sort_by(|a, b| a.0.cmp(&b.0));
    plan
}

const USAGE: &str = "Usage: russet migrate [options]

Copy AutoPkg's preferences into Russet's preferences. AutoPkg's preferences
and folders are left unchanged. When AutoPkg's search or override folders
are unset, AutoPkg's default folders are copied.

Options:
  -n, --dry-run  Show what would change without writing anything
  -f, --force    Replace Russet values that differ from AutoPkg's
  -h, --help     Show this help message and exit";

pub fn migrate_preferences(args: &[String]) -> Result<i32, String> {
    let mut dry_run = false;
    let mut force = false;
    for arg in args {
        match arg.as_str() {
            "-n" | "--dry-run" => dry_run = true,
            "-f" | "--force" => force = true,
            "-h" | "--help" => {
                autopkg_platform::text_println!("{USAGE}");
                return Ok(0);
            }
            other => {
                autopkg_platform::text_eprintln!(
                    "{USAGE}\n\nrusset migrate: error: unexpected argument: {other}"
                );
                return Ok(2);
            }
        }
    }
    let (autopkg, defaults) = with_autopkg_defaults(&autopkg_layer()?.values, |folder| {
        expand(folder).is_ok_and(|path| path.is_dir())
    });
    let mut russet = russet_layer()?;
    let plan = plan_migration(&autopkg, &russet.values, force);
    if plan.is_empty() {
        autopkg_platform::text_println!("AutoPkg has no preferences to migrate.");
        return Ok(0);
    }
    let width = plan.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    for (key, outcome) in &plan {
        let note = if defaults.contains(&key.as_str()) {
            " (AutoPkg's default)"
        } else {
            ""
        };
        autopkg_platform::text_println!("  {key:width$}  {}{note}", outcome.label());
    }
    let count = |wanted: Outcome| plan.iter().filter(|(_, o)| *o == wanted).count();
    let keys: Vec<&str> = plan
        .iter()
        .filter(|(_, outcome)| outcome.writes())
        .map(|(key, _)| key.as_str())
        .collect();
    autopkg_platform::text_println!(
        "\n{} copied, {} replaced, {} unchanged, {} conflicting.",
        count(Outcome::Copied),
        count(Outcome::Replaced),
        count(Outcome::Unchanged),
        count(Outcome::Conflict)
    );
    if count(Outcome::Conflict) > 0 {
        autopkg_platform::text_println!(
            "Russet kept its own value for each conflict. Run with --force to use AutoPkg's."
        );
    }
    if dry_run {
        autopkg_platform::text_println!("Dry run: nothing was written.");
        return Ok(0);
    }
    if keys.is_empty() {
        return Ok(0);
    }
    for key in &keys {
        let value: &Value = &autopkg[*key];
        russet.values.insert((*key).into(), value.clone());
    }
    russet.save(&keys)?;
    autopkg_platform::text_println!(
        "Wrote {} preference(s) to {}.",
        keys.len(),
        russet.location()
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary(values: &[(&str, &str)]) -> Dictionary {
        Dictionary::from_iter(values.iter().map(|(k, v)| (*k, Value::String((*v).into()))))
    }

    #[test]
    fn copies_missing_skips_equal_and_reports_conflicts() {
        let autopkg = dictionary(&[("NEW", "a"), ("SAME", "b"), ("DIFFERENT", "c")]);
        let russet = dictionary(&[("SAME", "b"), ("DIFFERENT", "russet"), ("OWN", "d")]);
        assert_eq!(
            plan_migration(&autopkg, &russet, false),
            [
                ("DIFFERENT".to_owned(), Outcome::Conflict),
                ("NEW".to_owned(), Outcome::Copied),
                ("SAME".to_owned(), Outcome::Unchanged),
            ]
        );
    }

    #[test]
    fn force_replaces_conflicts_only() {
        let autopkg = dictionary(&[("SAME", "b"), ("DIFFERENT", "c")]);
        let russet = dictionary(&[("SAME", "b"), ("DIFFERENT", "russet")]);
        assert_eq!(
            plan_migration(&autopkg, &russet, true),
            [
                ("DIFFERENT".to_owned(), Outcome::Replaced),
                ("SAME".to_owned(), Outcome::Unchanged),
            ]
        );
    }

    #[test]
    fn nothing_to_migrate() {
        assert!(plan_migration(&Dictionary::new(), &dictionary(&[("A", "b")]), false).is_empty());
    }

    #[test]
    fn unset_folders_get_autopkg_defaults_when_they_exist() {
        let overrides = |f: &str| f == "~/Library/AutoPkg/RecipeOverrides";
        let (values, added) = with_autopkg_defaults(&Dictionary::new(), overrides);
        assert_eq!(added, ["RECIPE_OVERRIDE_DIRS"]);
        assert_eq!(
            values["RECIPE_OVERRIDE_DIRS"],
            Value::Array(vec!["~/Library/AutoPkg/RecipeOverrides".into()])
        );
        assert!(!values.contains_key("RECIPE_SEARCH_DIRS"));

        let empty = Dictionary::from_iter([("RECIPE_SEARCH_DIRS", Value::Array(vec![]))]);
        let (values, added) = with_autopkg_defaults(&empty, |_| true);
        assert_eq!(added, ["RECIPE_SEARCH_DIRS", "RECIPE_OVERRIDE_DIRS"]);
        assert_eq!(values["RECIPE_SEARCH_DIRS"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn set_folders_and_missing_defaults_are_left_alone() {
        let set = dictionary(&[("RECIPE_OVERRIDE_DIRS", "/srv/overrides")]);
        let (values, added) = with_autopkg_defaults(&set, |_| true);
        assert_eq!(added, ["RECIPE_SEARCH_DIRS"]);
        assert_eq!(
            values["RECIPE_OVERRIDE_DIRS"].as_string(),
            Some("/srv/overrides")
        );
        assert!(with_autopkg_defaults(&Dictionary::new(), |_| false)
            .1
            .is_empty());
    }
}
