use plist::{Dictionary, Value};
mod audit;
mod cache;
mod discovery;
mod jobs;
mod manage;
mod options;
mod search;
use std::{
    env,
    io::{self, Read},
    path::PathBuf,
};

fn contract() -> serde_json::Value {
    autopkg_processors::contract().clone()
}

fn dictionary(path: &str) -> Result<Dictionary, String> {
    manage::load_preferences(Some(path))
}

fn list_strings(values: &Dictionary, key: &str) -> Result<Vec<String>, String> {
    match values.get(key) {
        None => Ok(vec![]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_string()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{key} must contain strings"))
            })
            .collect(),
        _ => Err(format!("{key} must be an array")),
    }
}
fn read_recipe_list(path: &str) -> Result<Dictionary, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(b"bplist") || bytes.starts_with(b"<?xml") || bytes.starts_with(b"<plist") {
        if let Ok(value) = Value::from_reader(io::Cursor::new(&bytes)) {
            if let Some(dictionary) = value.into_dictionary() {
                return Ok(dictionary);
            }
        }
    }
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok(Dictionary::from_iter([(
        "recipes",
        Value::Array(
            text.lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|s| Value::String(s.into()))
                .collect(),
        ),
    )]))
}

fn print_variables(order: &serde_json::Value, indent: usize) {
    if let Some(entries) = order.as_array() {
        for entry in entries {
            let key = entry[0].as_str().unwrap();
            if entry[1].is_array() {
                autopkg_platform::text_println!("{} {key}:", " ".repeat(indent));
                print_variables(&entry[1], indent + 2);
            } else {
                autopkg_platform::text_println!(
                    "{} {key}: {}",
                    " ".repeat(indent),
                    if entry[1]["dynamic"].as_str() == Some("signtool_default_path") {
                        autopkg_platform::signature::signtool_default_path()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "None".into())
                    } else {
                        entry[1]["display"].as_str().unwrap().to_owned()
                    }
                );
            }
        }
    }
}

fn processor_run(args: &[String]) -> Result<i32, String> {
    if args.len() != 1 {
        return Err("Usage: russet processor-run NAME < input.plist > output.plist".into());
    }
    let name = &args[0];
    let manifest = contract();
    if manifest["processors"].get(name).is_none() {
        return Err(format!("Unknown processor '{name}'"));
    }
    let mut bytes = Vec::new();
    io::stdin()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let mut environment = Value::from_reader(io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?
        .into_dictionary()
        .ok_or("Processor input must be a plist dictionary")?;
    if environment
        .get("force_munki_repo_lib")
        .and_then(Value::as_boolean)
        == Some(true)
    {
        return Err("Python Munki repository libraries are unsupported".into());
    }
    let preferences = manage::load_preferences(None)?;
    if let Err(error) = autopkg_processors::execute_standalone_with_preferences(
        name,
        &mut environment,
        &preferences,
    ) {
        match error.kind {
            autopkg_processors::FailureKind::Processor => return Err(error.message),
            autopkg_processors::FailureKind::Unexpected => {
                autopkg_platform::text_eprintln!("{}", error.message);
                return Ok(1);
            }
        }
    }
    environment.retain(|_, value| !value.is_null());
    Value::Dictionary(environment)
        .to_writer_xml(io::stdout())
        .map_err(|e| e.to_string())?;
    Ok(0)
}

fn persist_receipt(source: &std::path::Path, environment: &Dictionary, receipt: &[Value]) {
    let Some(root) = environment
        .get("RECIPE_CACHE_DIR")
        .and_then(Value::as_string)
    else {
        return;
    };
    let directory = std::path::Path::new(root).join("receipts");
    let name = source.file_stem().unwrap_or_default().to_string_lossy();
    let path = directory.join(format!(
        "{name}-receipt-{}.plist",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    let outcome = std::fs::create_dir_all(&directory)
        .map_err(|e| e.to_string())
        .and_then(|()| autopkg_engine::report::write_receipt(receipt, &path));
    if let Err(error) = outcome {
        autopkg_platform::text_eprintln!("Can't write receipt to {}: {error}", path.display());
    } else if environment
        .get("verbose")
        .and_then(Value::as_unsigned_integer)
        .unwrap_or(0)
        > 0
    {
        autopkg_platform::text_println!("Receipt written to {}", path.display());
    }
}

fn run_arguments(args: &[String]) -> Result<Vec<String>, String> {
    let values = [
        "--pre",
        "--preprocessor",
        "--post",
        "--postprocessor",
        "--prefs",
        "--recipe-list",
        "--search-dir",
        "--override-dir",
        "--report-plist",
        "--pkg",
        "--key",
        "--jobs",
    ];
    let flags = [
        "--check",
        "--verbose",
        "--quiet",
        "--help",
        "--ignore-parent-trust-verification-errors",
    ];
    let mut result = vec![];
    let mut expecting = false;
    let mut positional = false;
    for arg in args {
        if positional || expecting {
            result.push(arg.clone());
            expecting = false;
            continue;
        }
        if arg == "--" {
            positional = true;
            result.push(arg.clone());
            continue;
        }
        if arg.starts_with("--") {
            let (name, value) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(a, b)| (a, Some(b)));
            let matches: Vec<_> = values
                .iter()
                .chain(flags.iter())
                .filter(|known| known.starts_with(name))
                .copied()
                .collect();
            let name = if values.contains(&name) || flags.contains(&name) {
                name
            } else if matches.len() == 1 {
                matches[0]
            } else if matches.len() > 1 {
                return Err(format!("Ambiguous option: {name}"));
            } else {
                name
            };
            result.push(name.into());
            if let Some(value) = value {
                if !values.contains(&name) {
                    return Err(format!("{name} does not take a value"));
                }
                result.push(value.into());
            } else {
                expecting = values.contains(&name);
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            let chars = arg[1..].char_indices();
            for (offset, c) in chars {
                result.push(format!("-{c}"));
                if "kldpj".contains(c) {
                    let tail = &arg[offset + 2..];
                    if tail.is_empty() {
                        expecting = true
                    } else {
                        result.push(tail.into())
                    }
                    break;
                }
            }
        } else {
            result.push(arg.clone());
        }
    }
    Ok(result)
}

fn run(verb: &str, args: &[String]) -> Result<i32, String> {
    let args = run_arguments(args)?;
    let mut initial = manage::load_preferences(None)?;
    let mut keys = Dictionary::new();
    let mut paths = Vec::new();
    let mut override_paths = Vec::new();
    let mut report_path = None;
    let mut recipes = Vec::new();
    let mut recipe_list = Dictionary::new();
    let mut pre = Vec::new();
    let mut post = Vec::new();
    let mut check_only = false;
    let mut ignore_trust = false;
    let mut pkg = None;
    let mut jobs = None;
    let mut verbose: u64 = 0;
    let debug = env::var_os("AUTOPKG_RS_DEBUG").is_some();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-c" | "--check" => check_only = true,
            "-v" | "--verbose" => verbose += 1,
            "--ignore-parent-trust-verification-errors" => ignore_trust = true,
            "-h" | "--help" => {
                autopkg_platform::text_println!("Usage: russet {verb} [options] [recipe ...]\n  --pre/--preprocessor NAME   Repeatable preprocessor\n  --post/--postprocessor NAME Repeatable postprocessor\n  -c/--check                 Check for new downloads\n  --ignore-parent-trust-verification-errors\n  -k/--key KEY=VALUE         Repeatable input override\n  -l/--recipe-list PATH      Text or plist recipe list\n  -p/--pkg PATH              Existing package or disk image\n  --report-plist PATH        Save summary report\n  -v/--verbose              Repeat for more diagnostics\n  -q/--quiet                Disable recipe search suggestions\n  -d/--search-dir DIRECTORY  Repeatable recipe directory\n  --override-dir DIRECTORY  Repeatable override directory\n  --prefs PATH              Preference file\n  -j/--jobs N               Recipes to run at once; 0 is one per CPU");
                return Ok(0);
            }
            "-q" | "--quiet" => {}
            "--pre" | "--preprocessor" => {
                pre.push(iter.next().ok_or("--pre requires a processor")?.clone())
            }
            "--post" | "--postprocessor" => {
                post.push(iter.next().ok_or("--post requires a processor")?.clone())
            }
            "--prefs" => {
                initial =
                    manage::load_preferences(Some(iter.next().ok_or("--prefs requires a path")?))?
            }
            "-l" | "--recipe-list" => {
                recipe_list = read_recipe_list(iter.next().ok_or("--recipe-list requires a path")?)?
            }
            "-d" | "--search-dir" => paths.push(PathBuf::from(
                iter.next().ok_or("Search directory requires a path")?,
            )),
            "--override-dir" => override_paths.push(PathBuf::from(
                iter.next().ok_or("--override-dir requires a path")?,
            )),
            "--report-plist" => {
                report_path = Some(PathBuf::from(
                    iter.next().ok_or("--report-plist requires a path")?,
                ))
            }
            "-p" | "--pkg" => pkg = Some(iter.next().ok_or("--pkg requires a path")?.clone()),
            "-j" | "--jobs" => {
                let value = iter.next().ok_or("--jobs requires a number")?;
                let Some(count) = jobs::parse(value) else {
                    return options::usage_failure(
                        verb,
                        Some(&format!(
                            "--jobs must be a whole number of 0 or more, not {value:?}"
                        )),
                        2,
                    );
                };
                jobs = Some(count);
            }
            "-k" | "--key" => {
                let pair = iter.next().ok_or("--key requires KEY=VALUE")?;
                let (key, value) = pair.split_once('=').ok_or("--key requires KEY=VALUE")?;
                keys.insert(key.into(), Value::String(value.into()));
            }
            "--" => {
                recipes.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(format!(
                    "Unsupported run option '{flag}' in this development build"
                ))
            }
            _ => recipes.push(arg.clone()),
        }
    }
    if verb == "install" {
        recipes = recipes
            .into_iter()
            .filter_map(|name| {
                match std::path::Path::new(&name)
                    .extension()
                    .and_then(|v| v.to_str())
                {
                    None => Some(format!("{name}.install")),
                    Some("install") => Some(name),
                    _ => {
                        autopkg_platform::text_eprintln!(
                            "Can't install with a non-install recipe: {name}"
                        );
                        None
                    }
                }
            })
            .collect();
    }
    recipes.extend(list_strings(&recipe_list, "recipes")?);
    if pre.is_empty() {
        pre = list_strings(&recipe_list, "preprocessors")?;
    }
    if post.is_empty() {
        post = list_strings(&recipe_list, "postprocessors")?;
    }
    let processor_preferences = initial.clone();
    let mut runtime = Dictionary::new();
    for (key, value) in env::vars() {
        if let Some(key) = key.strip_prefix("AUTOPKG_") {
            if verbose > 1 {
                autopkg_platform::text_println!("Using environment var AUTOPKG_{key}={value}");
            }
            runtime.insert(key.into(), value.into());
        }
    }
    for (key, value) in recipe_list {
        if !matches!(key.as_str(), "recipes" | "preprocessors" | "postprocessors") {
            runtime.insert(key, value);
        }
    }
    runtime.extend(keys);
    if let Some(value) = &pkg {
        runtime.insert("PKG".into(), Value::String(value.clone()));
    }
    let keys = runtime;
    initial.insert("verbose".into(), Value::Integer(verbose.into()));
    if pkg.is_some() && recipes.len() > 1 {
        return Err("-p/--pkg option can't be used with multiple recipes!".into());
    }
    if recipes.is_empty() {
        return options::usage_failure(verb, None, 255);
    }
    let cache_root = cache::root(&initial)?;
    initial.insert(
        "CACHE_DIR".into(),
        cache_root.to_string_lossy().into_owned().into(),
    );
    let (configured_search, configured_overrides) = manage::recipe_directories(&initial)?;
    if paths.is_empty() {
        paths = configured_search;
    }
    if override_paths.is_empty() {
        override_paths = configured_overrides;
    }
    let policy = autopkg_engine::trust::TrustPolicy {
        override_dirs: override_paths.clone(),
        repository_dirs: manage::repository_dirs(&initial)?,
    };
    let mut search_paths = override_paths;
    search_paths.extend(paths);

    if recipes
        .iter()
        .any(|name| !std::path::Path::new(name).is_file())
    {
        manage::ensure_recipe_map(&initial)?;
    }
    // Resolve and validate all recipes before any recipe can mutate the filesystem.
    let mut loaded = recipes
        .iter()
        .map(|path| {
            let path = autopkg_engine::resolve_recipe(&PathBuf::from(path), &search_paths)?;
            let has_trust =
                autopkg_engine::read_recipe(&path)?.contains_key("ParentRecipeTrustInfo");
            let fail_without_trust = keys
                .get("FAIL_RECIPES_WITHOUT_TRUST_INFO")
                .or_else(|| initial.get("FAIL_RECIPES_WITHOUT_TRUST_INFO"))
                .is_some_and(|v| match v {
                    Value::Null => false,
                    Value::Boolean(b) => *b,
                    Value::String(s) => !s.is_empty(),
                    Value::Integer(n) => n.as_signed() != Some(0),
                    _ => true,
                });
            if !ignore_trust && fail_without_trust && !has_trust {
                return Err(format!(
                    "{} is missing parent recipe trust information",
                    path.display()
                ));
            }
            if ignore_trust {
                autopkg_engine::load_unverified_recipe(&path, &search_paths)
            } else if has_trust {
                autopkg_engine::trust::load_verified_recipe(&path, &search_paths, &policy)
            } else {
                autopkg_engine::load_recipe(&path, &search_paths)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    for recipe in &mut loaded {
        recipe.add_processors(&pre, &post);
        if check_only {
            *recipe = recipe.check_phase()?;
        }
    }
    let jobs = match jobs {
        Some(count) => count,
        None => jobs::preference(&keys, &initial)?.unwrap_or(1),
    };
    let context = RunContext {
        initial: &initial,
        keys: &keys,
        search_paths: &search_paths,
        options: autopkg_engine::RunOptions {
            check_only,
            preferences: Some(processor_preferences),
        },
        debug,
    };
    // Each recipe's cache folder. Recipes that share one never run at the
    // same time.
    let mut caches = Vec::new();
    for recipe in &loaded {
        let inputs = context.inputs(recipe)?;
        autopkg_engine::validate_recipe(recipe, &inputs)?;
        let root = inputs
            .get("CACHE_DIR")
            .and_then(Value::as_string)
            .unwrap_or_default();
        caches.push(
            autopkg_engine::cache::recipe_cache_path(
                std::path::Path::new(root),
                &recipe.identifier,
            )
            .unwrap_or_else(|_| PathBuf::from(root).join(&recipe.identifier)),
        );
    }
    let results_path = initial
        .get("CACHE_DIR")
        .and_then(Value::as_string)
        .map(|root| PathBuf::from(root).join("autopkg_results.plist"));
    if let Some(path) = &results_path {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        if let Err(error) = autopkg_engine::report::write_run_results(&[], path) {
            autopkg_platform::text_eprintln!("Can't write results to {}: {error}", path.display());
        }
    }
    // A recipe that rebuilds Munki catalogs waits for the imports listed
    // before it.
    let barriers: Vec<_> = loaded
        .iter()
        .map(|recipe| {
            recipe.process.iter().any(|step| {
                autopkg_processors::canonical_name(&step.processor)
                    .rsplit('/')
                    .next()
                    == Some("MakeCatalogsProcessor")
            })
        })
        .collect();
    let stages = jobs::stages(&caches, &barriers);
    let workers = jobs::workers(jobs);
    let prefixed = workers > 1 && stages.iter().any(|stage| stage.len() > 1);
    let outcomes = jobs::run(
        &stages,
        workers,
        |index| {
            let _prefix =
                prefixed.then(|| autopkg_platform::text_output::prefix_scope(&recipes[index]));
            context.run_one(&loaded[index], &recipes[index])
        },
        |outcomes| {
            if let Some(path) = &results_path {
                let receipts: Vec<_> = outcomes
                    .iter()
                    .flatten()
                    .map(|outcome| outcome.receipt.clone())
                    .collect();
                if let Err(error) = autopkg_engine::report::write_run_results(&receipts, path) {
                    autopkg_platform::text_eprintln!(
                        "Can't write results to {}: {error}",
                        path.display()
                    );
                }
            }
        },
    )?;
    let mut report = autopkg_engine::report::Report::default();
    for ((outcome, recipe), requested_name) in outcomes.iter().zip(&loaded).zip(&recipes) {
        report.add_receipt(&outcome.receipt)?;
        if let Some((message, traceback)) = &outcome.failure {
            report.add_failure(requested_name, Some(&recipe.identifier), message, traceback);
        }
    }
    print_summary(&report)?;
    if let Some(path) = report_path {
        report.write(&path)?;
        autopkg_platform::text_println!("\nReport plist saved to {}.", path.display());
    }
    Ok(if report.failures.is_empty() { 0 } else { 70 })
}

/// What one recipe's run left: its receipt and, if it failed, the message
/// and traceback for the report.
struct Outcome {
    receipt: Vec<Value>,
    failure: Option<(String, String)>,
}

/// Everything recipes in one `run` share.
struct RunContext<'a> {
    initial: &'a Dictionary,
    keys: &'a Dictionary,
    search_paths: &'a [PathBuf],
    options: autopkg_engine::RunOptions,
    debug: bool,
}

impl RunContext<'_> {
    /// A recipe's inputs: preferences, then the recipe's own input, then
    /// keys from the command line, the recipe list, and the environment.
    fn inputs(&self, recipe: &autopkg_engine::Recipe) -> Result<Dictionary, String> {
        let mut inputs = self.initial.clone();
        inputs.insert("AUTOPKG_VERSION".into(), "3.0.0".into());
        inputs.insert(
            "PARENT_RECIPES".into(),
            parent_paths(recipe, self.search_paths)?,
        );
        inputs.extend(recipe.input.clone());
        inputs.extend(self.keys.clone());
        let root = cache::root_with_override(self.initial, inputs.get("CACHE_DIR"))?;
        inputs.insert(
            "CACHE_DIR".into(),
            root.to_string_lossy().into_owned().into(),
        );
        Ok(inputs)
    }

    /// Runs one recipe and saves its receipt. An `Err` stops the whole run.
    fn run_one(
        &self,
        recipe: &autopkg_engine::Recipe,
        requested_name: &str,
    ) -> Result<Outcome, String> {
        autopkg_platform::text_println!("Processing {requested_name}...");
        if !autopkg_engine::read_recipe(&recipe.source)?.contains_key("ParentRecipeTrustInfo") {
            autopkg_platform::text_eprintln!("WARNING: {requested_name} is missing trust info and FAIL_RECIPES_WITHOUT_TRUST_INFO is not set. Proceeding...");
        }
        if self.debug {
            autopkg_platform::text_eprintln!(
                "Running {} ({})",
                recipe.identifier,
                recipe.source.display()
            );
        }
        let inputs = self.inputs(recipe)?;
        let outcome = match autopkg_engine::run_recipe_detailed(recipe, inputs, &self.options) {
            Ok(result) => {
                persist_receipt(
                    std::path::Path::new(requested_name),
                    &result.environment,
                    &result.receipt,
                );
                if self.debug {
                    autopkg_platform::text_eprintln!(
                        "Executed {} processor(s); stopped={}",
                        result.executed.len(),
                        result.stopped
                    );
                }
                Outcome {
                    receipt: result.receipt,
                    failure: None,
                }
            }
            Err(error) => {
                autopkg_platform::text_eprintln!("Failed.");
                persist_receipt(
                    std::path::Path::new(requested_name),
                    &error.partial.environment,
                    &error.partial.receipt,
                );
                let traceback = format!(
                    "{}\nNative Rust backtrace:\n{}",
                    error.message,
                    std::backtrace::Backtrace::force_capture()
                );
                Outcome {
                    receipt: error.partial.receipt,
                    failure: Some((error.message, traceback)),
                }
            }
        };
        // A receipt the report can't summarize stops the run here, as it
        // did when recipes always ran one at a time.
        autopkg_engine::report::Report::default().add_receipt(&outcome.receipt)?;
        Ok(outcome)
    }
}

fn parent_paths(
    recipe: &autopkg_engine::Recipe,
    search_paths: &[PathBuf],
) -> Result<Value, String> {
    let mut parents = Vec::new();
    let mut source = recipe.source.clone();
    loop {
        let data = autopkg_engine::read_recipe(&source)?;
        let Some(parent) = data
            .get("ParentRecipe")
            .or_else(|| data.get("Recipe"))
            .and_then(Value::as_string)
        else {
            break;
        };
        let mut dirs = search_paths.to_vec();
        if let Some(directory) = source.parent() {
            dirs.push(directory.to_owned());
        }
        source = autopkg_engine::resolve_recipe(std::path::Path::new(parent), &dirs)?
            .canonicalize()
            .map_err(|e| e.to_string())?;
        source = autopkg_engine::normal_recipe_path(source);
        parents.push(Value::from(source.to_string_lossy().into_owned()));
    }
    Ok(Value::Array(parents))
}

fn print_summary(report: &autopkg_engine::report::Report) -> Result<(), String> {
    if !report.failures.is_empty() {
        autopkg_platform::text_println!("\nThe following recipes failed:");
        for failure in &report.failures {
            autopkg_platform::text_println!(
                "    {}\n        {}",
                failure
                    .get("recipe")
                    .and_then(Value::as_string)
                    .unwrap_or(""),
                failure
                    .get("message")
                    .and_then(Value::as_string)
                    .unwrap_or("")
            );
        }
    }
    if report.summary_results.is_empty() {
        autopkg_platform::text_println!("\nNothing downloaded, packaged or imported.");
        return Ok(());
    }
    for (_, value) in &report.summary_results {
        let data = value.as_dictionary().ok_or("Invalid summary")?;
        autopkg_platform::text_println!(
            "\n{}",
            data.get("summary_text")
                .and_then(Value::as_string)
                .unwrap_or("")
        );
        let headers = data["header"].as_array().ok_or("Invalid headers")?;
        let names = headers
            .iter()
            .map(|v| v.as_string().ok_or("Invalid header"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rows = vec![
            names
                .iter()
                .map(|s| {
                    s.replace('_', " ")
                        .split_whitespace()
                        .map(|w| {
                            let mut c = w.chars();
                            c.next()
                                .map(|first| first.to_uppercase().collect::<String>() + c.as_str())
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>(),
            names.iter().map(|s| "-".repeat(s.len())).collect(),
        ];
        for row in data["data_rows"].as_array().ok_or("Invalid data rows")? {
            let row = row.as_dictionary().ok_or("Invalid data row")?;
            rows.push(
                names
                    .iter()
                    .map(|name| {
                        row.get(name)
                            .and_then(Value::as_string)
                            .unwrap_or("")
                            .to_owned()
                    })
                    .collect(),
            );
        }
        let widths = (0..names.len())
            .map(|i| {
                rows.iter()
                    .map(|row| row[i].chars().count())
                    .max()
                    .unwrap_or(0)
                    + 2
            })
            .collect::<Vec<_>>();
        for row in rows {
            autopkg_platform::text_print!("    ");
            for (cell, width) in row.iter().zip(&widths) {
                autopkg_platform::text_print!("{cell:width$}");
            }
            autopkg_platform::text_println!();
        }
    }
    Ok(())
}
fn dispatch(args: &[String]) -> Result<i32, String> {
    let original_verb = args.first().map(String::as_str);
    let known = original_verb.is_some_and(|verb| {
        contract()["cli"]["subcommands"].get(verb).is_some()
            || matches!(verb, "processor-run" | "generate-map")
    });
    if !known || original_verb == Some("help") {
        return Ok(options::top_help(original_verb));
    }
    let normalized;
    let args = match options::parse(original_verb.unwrap(), &args[1..]) {
        options::Parsed::Exit(code) => return Ok(code),
        options::Parsed::Arguments(rest) => {
            normalized = std::iter::once(args[0].clone())
                .chain(rest)
                .collect::<Vec<_>>();
            &normalized
        }
    };
    let verb = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    match verb {
        "list-recipes" | "info" => return discovery::run(verb, rest),
        "clear-cache" => return cache::run(rest),
        "new-recipe" => return manage::new_recipe(rest),
        "audit" => return audit::run(rest),
        "search" => return search::run(rest),
        "repo-add"
        | "repo-delete"
        | "repo-list"
        | "list-repos"
        | "repo-update"
        | "make-override"
        | "update-trust-info"
        | "verify-trust-info"
        | "generate-map"
        | "generate-recipe-map" => {
            return manage::run(
                if verb == "generate-map" {
                    "generate-recipe-map"
                } else {
                    verb
                },
                rest,
            )
        }
        "version" => autopkg_platform::text_println!("3.0.0"),
        "list-processors" | "processor-list" => {
            for name in contract()["processors"]
                .as_object()
                .ok_or("Invalid manifest")?
                .keys()
            {
                autopkg_platform::text_println!("{name}");
            }
        }
        "processor-info" => {
            let rest = options::operands(verb, rest);
            if rest.len() != 1 {
                autopkg_platform::text_eprintln!("Need exactly one processor name");
                return Ok(255);
            }
            let data = contract();
            let Some(manifest) = data["processors"].get(&rest[0]) else {
                autopkg_platform::text_eprintln!("Unknown processor '{}'", rest[0]);
                return Ok(255);
            };
            let ordering = autopkg_processors::processor_order();
            let mut ordering = ordering[&rest[0]].clone();
            if rest[0] == "Unarchiver" {
                for entry in ordering["input_variables"].as_array_mut().unwrap() {
                    if entry[0] == "USE_PYTHON_NATIVE_EXTRACTOR" {
                        for field in entry[1].as_array_mut().unwrap() {
                            if field[0] == "default" {
                                field[1]["display"] =
                                    (if cfg!(windows) { "True" } else { "False" }).into();
                            }
                        }
                    }
                }
            }
            let _ = manifest;
            autopkg_platform::text_println!(
                "Description: {}",
                ordering["description"]["display"].as_str().unwrap()
            );
            autopkg_platform::text_println!("Input variables:");
            print_variables(&ordering["input_variables"], 2);
            autopkg_platform::text_println!("Output variables:");
            print_variables(&ordering["output_variables"], 2);
        }
        "processor-run" => return processor_run(rest),
        "run" | "install" => return run(verb, rest),
        "help" | "--help" | "-h" => {
            autopkg_platform::text_println!("Usage: russet VERB [options]\n\nDevelopment build. The Python executable remains the release implementation.\n\nImplemented verbs:\n  version\n  list-processors (processor-list)\n  processor-info NAME\n  processor-run NAME < input.plist > output.plist\n  list-recipes [--plist] [-i] [-p]\n  info [RECIPE ...]\n  clear-cache RECIPE ...\n  repo-add URL ...\n  repo-delete REPOSITORY ...\n  repo-list (list-repos)\n  repo-update [REPOSITORY ...]\n  audit RECIPE ...\n  search SEARCH_TERM\n  new-recipe PATH\n  make-override RECIPE\n  update-trust-info RECIPE ...\n  verify-trust-info RECIPE ...\n  generate-recipe-map\n  install RECIPE ...\n  run [-l RECIPE_LIST] [-c] [-v] [-k KEY=VALUE] [-d DIRECTORY] [--prefs FILE] RECIPE ...\n\nOn macOS, as root:\n  --install-helpers    Load the launchd jobs PkgCreator and Installer need\n  --uninstall-helpers  Unload and remove those launchd jobs");
        }
        _ => {
            return Err(format!(
                "Command '{verb}' is not implemented in this development build"
            ))
        }
    }
    Ok(0)
}

/// Map `--server` and `--installd` to the privileged helper that launchd starts.
fn helper_service(arguments: &[String]) -> Option<Result<autopkg_helpers::Service, String>> {
    let service = match arguments.first().map(String::as_str)? {
        "--server" => autopkg_helpers::Service::Packaging,
        "--installd" => autopkg_helpers::Service::Installation,
        _ => return None,
    };
    Some(if arguments.len() == 1 {
        Ok(service)
    } else {
        Err(format!("Usage: russet {}", arguments[0]))
    })
}

type HelperSetup = fn() -> Result<(), String>;

/// Map `--install-helpers` and `--uninstall-helpers` to setting up or removing
/// the helpers' launchd jobs.
fn helper_setup(arguments: &[String]) -> Option<Result<HelperSetup, String>> {
    let action: HelperSetup = match arguments.first().map(String::as_str)? {
        "--install-helpers" => autopkg_helpers::install_launchd_jobs,
        "--uninstall-helpers" => autopkg_helpers::uninstall_launchd_jobs,
        _ => return None,
    };
    Some(if arguments.len() == 1 {
        Ok(action)
    } else {
        Err(format!("Usage: russet {}", arguments[0]))
    })
}

fn main() {
    let arguments: Vec<_> = env::args().skip(1).collect();
    if let Some(action) = helper_setup(&arguments) {
        if let Err(error) = action.and_then(|action| action()) {
            autopkg_platform::text_eprintln!("{error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    if let Some(service) = helper_service(&arguments) {
        if let Err(error) = service.and_then(autopkg_helpers::run) {
            autopkg_platform::text_eprintln!("{error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }
    let standalone = arguments
        .first()
        .is_some_and(|verb| verb == "processor-run");
    // Releases native disk-image extractions before the process exits;
    // std::process::exit doesn't run destructors.
    let images = autopkg_platform::dmg::RecipeScope::process();
    let code = match autopkg_platform::backend::validate().and_then(|()| dispatch(&arguments)) {
        Ok(code) => code,
        Err(error) => {
            if standalone {
                autopkg_platform::text_eprintln!("ProcessorError: {error}");
            } else {
                autopkg_platform::text_eprintln!("{error}");
            }
            if standalone {
                10
            } else {
                1
            }
        }
    };
    drop(images);
    #[cfg(windows)]
    let code = if code == 255 { -1 } else { code };
    std::process::exit(code);
}

#[cfg(test)]
mod option_tests {
    use super::*;
    #[test]
    fn helper_flags_select_a_service_and_take_no_arguments() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            helper_service(&args(&["--server"])),
            Some(Ok(autopkg_helpers::Service::Packaging))
        ));
        assert!(matches!(
            helper_service(&args(&["--installd"])),
            Some(Ok(autopkg_helpers::Service::Installation))
        ));
        assert_eq!(
            helper_service(&args(&["--server", "extra"]))
                .unwrap()
                .unwrap_err(),
            "Usage: russet --server"
        );
        assert!(helper_service(&args(&["run", "--server"])).is_none());
        assert!(helper_service(&[]).is_none());
    }
    #[test]
    fn helper_setup_flags_take_no_arguments() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(matches!(
            helper_setup(&args(&["--install-helpers"])),
            Some(Ok(_))
        ));
        assert!(matches!(
            helper_setup(&args(&["--uninstall-helpers"])),
            Some(Ok(_))
        ));
        assert_eq!(
            helper_setup(&args(&["--install-helpers", "extra"]))
                .unwrap()
                .err(),
            Some("Usage: russet --install-helpers".to_owned())
        );
        assert!(helper_setup(&args(&["--server"])).is_none());
        assert!(helper_setup(&args(&["run", "--install-helpers"])).is_none());
    }
    #[test]
    fn optparse_forms_preserve_values_and_separator() {
        let args = [
            "-vc",
            "-kNAME=-vv",
            "--report-p=out.plist",
            "--pre",
            "-v",
            "--",
            "-vv",
        ]
        .map(str::to_owned);
        assert_eq!(
            run_arguments(&args).unwrap(),
            [
                "-v",
                "-c",
                "-k",
                "NAME=-vv",
                "--report-plist",
                "out.plist",
                "--pre",
                "-v",
                "--",
                "-vv"
            ]
        );
        assert!(run_arguments(&["--p=value".into()]).is_err());
        assert!(run_arguments(&["--verbose=yes".into()]).is_err());
    }
}
