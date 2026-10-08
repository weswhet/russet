//! Static recipe auditing. Processor code is never imported or executed.
use plist::Value as Plist;
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};
const CHECKS: [(&str, &str, &str); 7] = [
    ("missing_codesig", "MissingCodeSignatureVerifier", "warning"),
    ("sensitive_input", "sensitive_inputs", "error"),
    ("insecure_protocol", "http_urls", "warning"),
    ("path_safety", "path_safety_warnings", "warning"),
    ("weak_hash", "weak_hashes", "warning"),
    ("non_core_processor", "non_core_processors", "info"),
    ("modification_processor", "audit_processors", "info"),
];
fn rank(s: &str) -> Option<usize> {
    ["info", "warning", "error"].iter().position(|v| *v == s)
}
fn parent(s: &str) -> bool {
    s.replace('\\', "/").split('/').any(|p| p == "..")
}
fn absolute(s: &str) -> bool {
    let s = s.replace('\\', "/");
    s.starts_with('/') || s.as_bytes().get(1) == Some(&b':')
}
fn marker(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.contains("..")
}
fn inner(s: &str) -> Option<&str> {
    let lower = s.to_ascii_lowercase();
    [".dmg/", ".iso/"]
        .iter()
        .find_map(|ext| lower.find(ext).map(|i| &s[i + ext.len()..]))
}
fn outside(v: &Value) -> bool {
    let Some(s) = v.as_str().filter(|s| !s.is_empty()) else {
        return false;
    };
    if parent(s) {
        return true;
    }
    if inner(s).is_some() {
        return false;
    }
    let normalized = s.replace('\\', "/");
    if ["%RECIPE_CACHE_DIR%", "%CACHE_DIR%"]
        .iter()
        .any(|p| normalized == *p || normalized.starts_with(&format!("{p}/")))
    {
        return false;
    }
    if s.contains('%') {
        return absolute(s);
    }
    true
}
fn py(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        _ => v.to_string(),
    }
}
fn variable_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn substitute(v: &Value, env: &Map<String, Value>, depth: usize) -> Value {
    if depth > 5 {
        return v.clone();
    }
    match v {
        Value::String(s) => {
            if s.len() >= 2
                && s.starts_with('%')
                && s.ends_with('%')
                && variable_name(&s[1..s.len() - 1])
            {
                if let Some(value) = env.get(&s[1..s.len() - 1]) {
                    return substitute(value, env, depth + 1);
                }
            }
            let mut result = String::new();
            let mut rest = s.as_str();
            while let Some(start) = rest.find('%') {
                result.push_str(&rest[..start]);
                rest = &rest[start..];
                if let Some(end) = rest[1..].find('%').map(|v| v + 1) {
                    let key = &rest[1..end];
                    if variable_name(key) {
                        if let Some(value) = env.get(key) {
                            result.push_str(&py(value));
                        } else {
                            result.push_str(&rest[..=end]);
                        }
                        rest = &rest[end + 1..];
                        continue;
                    }
                }
                result.push('%');
                rest = &rest[1..];
            }
            result.push_str(rest);
            Value::String(result)
        }
        Value::Array(a) => Value::Array(a.iter().map(|v| substitute(v, env, depth + 1)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), substitute(v, env, depth + 1)))
                .collect(),
        ),
        _ => v.clone(),
    }
}
fn step_value(recipe: &Value, step: &Value, key: &str) -> Value {
    let mut env = recipe["Input"].as_object().cloned().unwrap_or_default();
    if let Some(args) = step["Arguments"].as_object() {
        env.extend(args.clone())
    }
    substitute(env.get(key).unwrap_or(&Value::Null), &env, 0)
}
fn strings(v: &Value, location: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::String(s) => out.push((location.into(), s.clone())),
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                strings(v, &format!("{location}[{i}]"), out)
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                strings(v, &format!("{location}.{k}"), out)
            }
        }
        _ => {}
    }
}
fn warning(out: &mut Vec<Value>, location: &str, reason: &str, value: &str) {
    if !out
        .iter()
        .any(|v| v["location"] == location && v["value"] == value)
    {
        out.push(json!({"location":location,"reason":reason,"value":value}));
    }
}
fn traversal(out: &mut Vec<Value>, v: &Value, location: &str) {
    match v {
        Value::String(s) if parent(s) => warning(
            out,
            location,
            "path contains parent-directory references",
            s,
        ),
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                traversal(out, v, &format!("{location}[{i}]"))
            }
        }
        _ => {}
    }
}
fn paths(recipe: &Value) -> Vec<Value> {
    let mut out = vec![];
    if let Some(id) = recipe
        .get("Identifier")
        .or_else(|| recipe["Input"].get("IDENTIFIER"))
        .and_then(Value::as_str)
    {
        if marker(id) {
            warning(
                &mut out,
                "Identifier",
                "contains path separators or parent-directory references",
                id,
            )
        }
    }
    let mut values = vec![];
    if let Some(input) = recipe["Input"].as_object() {
        for (k, v) in input {
            strings(v, &format!("Input.{k}"), &mut values)
        }
    }
    let steps = recipe["Process"].as_array().cloned().unwrap_or_default();
    for step in &steps {
        let processor = step["Processor"].as_str().unwrap_or("UnknownProcessor");
        if let Some(args) = step["Arguments"].as_object() {
            for (k, v) in args {
                strings(v, &format!("{processor}.{k}"), &mut values)
            }
        }
    }
    for (location, value) in values {
        if !value.contains("://")
            && inner(&value).is_some_and(|s| !s.is_empty() && (absolute(s) || parent(s)))
        {
            warning(&mut out,&location,"DMG pseudo-path contains an absolute path or parent-directory reference after the disk image boundary",&value)
        }
    }
    for step in &steps {
        let processor = step["Processor"].as_str().unwrap_or("");
        match processor {
            "Installer" => {
                let v = step_value(recipe, step, "pkg_path");
                if outside(&v) {
                    warning(&mut out,"Installer.pkg_path","package path is not explicitly inside AutoPkg cache or a mounted disk image",v.as_str().unwrap())
                }
            }
            "InstallFromDMG" => {
                if let Some(items) = step_value(recipe, step, "items_to_copy").as_array() {
                    for (i, item) in items.iter().enumerate() {
                        if let Some(source) = item["source_item"].as_str() {
                            if absolute(source) || parent(source) {
                                warning(
                                    &mut out,
                                    &format!("InstallFromDMG.items_to_copy[{i}].source_item"),
                                    "copy source may resolve outside the mounted disk image",
                                    source,
                                )
                            }
                        }
                        if outside(&item["destination_path"]) {
                            warning(&mut out,&format!("InstallFromDMG.items_to_copy[{i}].destination_path"),"privileged copy destination is not explicitly inside AutoPkg cache",item["destination_path"].as_str().unwrap())
                        }
                    }
                }
            }
            "PkgRootCreator" => {
                if let Some(dirs) = step_value(recipe, step, "pkgdirs").as_object() {
                    for dir in dirs.keys() {
                        if dir.contains("..") {
                            warning(
                                &mut out,
                                &format!("PkgRootCreator.pkgdirs[{dir}]"),
                                "pkgdirs key contains parent-directory references",
                                dir,
                            )
                        }
                    }
                }
            }
            "ChocolateyPackager" => {
                for key in ["id", "version"] {
                    if let Some(v) = step_value(recipe, step, key).as_str() {
                        if marker(v) {
                            warning(
                                &mut out,
                                &format!("ChocolateyPackager.{key}"),
                                "contains path separators or parent-directory references",
                                v,
                            )
                        }
                    }
                }
            }
            _ => {
                let keys: &[&str] = match processor {
                    "Copier" => &["source_path", "destination_path"],
                    "FileMover" => &["source", "target"],
                    "PathDeleter" => &["path_list"],
                    "Unarchiver" => &["archive_path", "destination_path"],
                    _ => &[],
                };
                for key in keys {
                    traversal(
                        &mut out,
                        &step_value(recipe, step, key),
                        &format!("{processor}.{key}"),
                    )
                }
            }
        }
    }
    out
}
fn analyze(recipe: &Value) -> Value {
    let mut out = Map::new();
    let mut sensitive = vec![];
    let mut urls = Map::new();
    if let Some(input) = recipe["Input"].as_object() {
        for (key, value) in input {
            if let Some(value) = value.as_str() {
                if value.starts_with("http:") || value.starts_with("ftp:") {
                    urls.entry("Input").or_insert_with(|| json!({}))[key] = value.into();
                }
                let lower = key.to_lowercase();
                let stem = lower.trim_end_matches(['_', '-']);

                if !["url", "uri", "endpoint", "server", "host", "address"]
                    .iter()
                    .any(|s| stem.ends_with(s))
                    && [
                        "password",
                        "passwd",
                        "secret",
                        "apikey",
                        "api_key",
                        "api-key",
                        "token",
                        "accesskey",
                        "access_key",
                        "access-key",
                        "privatekey",
                        "private_key",
                        "private-key",
                        "credential",
                        "bearer",
                    ]
                    .iter()
                    .any(|s| lower.contains(s))
                    && !value.is_empty()
                    && !value.contains('%')
                {
                    sensitive.push(json!({"location":format!("Input.{key}"),"reason":"appears to contain a hard-coded credential; supply it via a variable or an override instead"}));
                }
            }
        }
    }
    let steps = recipe["Process"].as_array().cloned().unwrap_or_default();
    let mut weak = vec![];
    let mut processors = vec![];
    for step in &steps {
        let processor = step["Processor"].as_str().unwrap_or("");
        processors.push(processor);
        if let Some(args) = step["Arguments"].as_object() {
            for (key, value) in args {
                if let Some(value) = value.as_str() {
                    if value.starts_with("http:") || value.starts_with("ftp:") {
                        let process = urls.entry("Process").or_insert_with(|| json!({}));
                        if process.get(processor).is_none() {
                            process[processor] = json!({});
                        }
                        process[processor][key] = value.into();
                    }
                }
            }
        }
        if processor == "ChocolateyPackager" {
            if let Some(algorithm) = step_value(recipe, step, "installer_checksum_type").as_str() {
                let algorithm = algorithm.to_lowercase();
                if ["md5", "sha1"].contains(&algorithm.as_str()) {
                    weak.push(json!({"location":"ChocolateyPackager.installer_checksum_type","algorithm":algorithm,"reason":format!("uses weak algorithm '{algorithm}'; prefer sha256 or stronger")}));
                }
            }
        }
    }
    if !urls.is_empty() {
        out.insert("http_urls".into(), urls.into());
    }
    let warnings = paths(recipe);
    if !warnings.is_empty() {
        out.insert("path_safety_warnings".into(), warnings.into());
    }
    if !weak.is_empty() {
        out.insert("weak_hashes".into(), weak.into());
    }
    if !sensitive.is_empty() {
        out.insert("sensitive_inputs".into(), sensitive.into());
    }
    if processors
        .iter()
        .any(|p| ["URLDownloader", "URLDownloaderPython"].contains(p))
        && !processors.contains(&"CodeSignatureVerifier")
    {
        out.insert("MissingCodeSignatureVerifier".into(), true.into());
    }
    let contract = super::contract();
    let noncore: Vec<_> = processors
        .iter()
        .filter(|p| contract["processors"].get(**p).is_none())
        .copied()
        .collect();
    let creators = [
        "AppPkgCreator",
        "ChocolateyPackager",
        "DmgCreator",
        "FlatPkgPacker",
        "PkgCreator",
    ];
    let mut mods: BTreeSet<_> = processors
        .iter()
        .filter(|p| creators.contains(p))
        .copied()
        .collect();
    for (i, p) in processors.iter().enumerate() {
        if [
            "Copier",
            "FileCreator",
            "FileMover",
            "PathDeleter",
            "PkgInfoCreator",
            "PlistEditor",
            "Symlinker",
        ]
        .contains(p)
            && processors[i + 1..]
                .iter()
                .any(|p| creators.contains(p) || noncore.contains(p))
        {
            mods.insert(p);
        }
    }
    if !mods.is_empty() || !noncore.is_empty() {
        out.insert("audit_processors".into(), json!(mods));
    }
    if !noncore.is_empty() {
        out.insert("non_core_processors".into(), json!(noncore));
    }
    out.into()
}
fn details(check: &str, value: &Value) -> Vec<String> {
    match check {
        "missing_codesig" => {
            vec!["No CodeSignatureVerifier step in a recipe that downloads.".into()]
        }
        "insecure_protocol" => {
            let mut out = vec![];
            if let Some(input) = value["Input"].as_object() {
                for (k, v) in input {
                    out.push(format!("Input.{k}: {}", py(v)))
                }
            }
            if let Some(process) = value["Process"].as_object() {
                for (p, args) in process {
                    if let Some(args) = args.as_object() {
                        for (k, v) in args {
                            out.push(format!("Process.{p}.{k}: {}", py(v)))
                        }
                    }
                }
            }
            out
        }
        "path_safety" | "weak_hash" | "sensitive_input" => value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                format!(
                    "{}: {}",
                    v["location"].as_str().unwrap(),
                    v["reason"].as_str().unwrap()
                )
            })
            .collect(),
        _ => value.as_array().unwrap().iter().map(py).collect(),
    }
}
fn findings(result: &Value) -> Vec<Value> {
    let mut out = vec![];
    for (check, key, severity) in CHECKS {
        if let Some(value) = result.get(key) {
            for detail in details(check, value) {
                out.push(json!({"severity":severity,"check":check,"detail":detail}));
            }
        }
    }
    out.sort_by(|a, b| {
        rank(b["severity"].as_str().unwrap())
            .cmp(&rank(a["severity"].as_str().unwrap()))
            .then(a["check"].as_str().cmp(&b["check"].as_str()))
    });
    out
}
fn load(path: &Path, dirs: &[PathBuf], active: &mut HashSet<PathBuf>) -> Result<Value, String> {
    let path = autopkg_engine::resolve_recipe(path, dirs)?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !active.insert(path.clone()) {
        return Err("Recipe inheritance cycle".into());
    }
    let own =
        serde_json::to_value(autopkg_engine::read_recipe(&path)?).map_err(|e| e.to_string())?;
    let mut merged = if let Some(parent) = own
        .get("ParentRecipe")
        .or_else(|| own.get("Recipe"))
        .and_then(Value::as_str)
    {
        let mut search = dirs.to_vec();
        search.push(path.parent().unwrap().into());
        let mut data = load(Path::new(parent), &search, active)?;
        let parent_path = data["RECIPE_PATH"].clone();
        data["PARENT_RECIPES"]
            .as_array_mut()
            .unwrap()
            .insert(0, parent_path);
        data
    } else {
        json!({"Input":{},"Process":[],"PARENT_RECIPES":[]})
    };
    if let Some(input) = own["Input"].as_object() {
        merged["Input"]
            .as_object_mut()
            .unwrap()
            .extend(input.clone());
    }
    if let Some(steps) = own["Process"].as_array() {
        merged["Process"]
            .as_array_mut()
            .unwrap()
            .extend(steps.clone());
    }
    if let Some(id) = own.get("Identifier") {
        merged["Identifier"] = id.clone()
    }
    merged["RECIPE_PATH"] = path.to_string_lossy().to_string().into();
    active.remove(&path);
    Ok(merged)
}
fn pretty(label: &str, value: &Value, indent: usize) {
    let prefix = "    ".repeat(indent);
    match value {
        Value::Object(items) => {
            if !label.is_empty() {
                autopkg_platform::text_println!("{prefix}{label}:");
            }
            for (key, value) in items {
                pretty(key, value, indent + 1)
            }
        }
        Value::Array(items) => {
            if !label.is_empty() {
                autopkg_platform::text_println!("{prefix}{label}:");
            }
            for value in items {
                pretty("", value, indent + 1)
            }
        }
        _ => {
            if label.is_empty() {
                autopkg_platform::text_println!("{prefix}{}", py(value))
            } else {
                autopkg_platform::text_println!("{prefix}{label}: {}", py(value))
            }
        }
    }
}
fn human(name: &str, recipe: &Value, result: &Value) {
    if result.as_object().unwrap().is_empty() {
        autopkg_platform::text_println!("{name}: no audit flags triggered.");
        return;
    }
    autopkg_platform::text_println!(
        "{name}\n    File path:        {}",
        recipe["RECIPE_PATH"].as_str().unwrap_or("")
    );
    if let Some(parents) = recipe["PARENT_RECIPES"]
        .as_array()
        .filter(|v| !v.is_empty())
    {
        autopkg_platform::text_println!(
            "    Parent recipe(s): {}",
            parents
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n                      ")
        );
    }
    let render_items = |key: &str, title: &str| {
        if let Some(items) = result[key].as_array() {
            autopkg_platform::text_println!("    {title}");
            for item in items {
                autopkg_platform::text_println!(
                    "        {}: {}",
                    item["location"].as_str().unwrap(),
                    item["reason"].as_str().unwrap()
                );
                if let Some(value) = item.get("value") {
                    autopkg_platform::text_println!("            {}", py(value));
                }
            }
        }
    };
    render_items(
        "sensitive_inputs",
        "The following inputs may contain hard-coded credentials:",
    );
    if result.get("MissingCodeSignatureVerifier").is_some() {
        autopkg_platform::text_println!("    Missing CodeSignatureVerifier");
    }
    if let Some(urls) = result.get("http_urls") {
        autopkg_platform::text_println!(
            "    The following insecure URLs were found in the recipe:"
        );
        if let Some(items) = urls.as_object() {
            for (key, value) in items {
                pretty(key, value, 2);
            }
        }
    }
    render_items(
        "path_safety_warnings",
        "The following path values should be more closely inspected:",
    );
    render_items(
        "weak_hashes",
        "The following weak hash algorithms were found in the recipe:",
    );
    for(key,title)in [("non_core_processors","The following processors are non-core and can execute arbitrary code, performing any action.\n    Be sure you understand what the processor does and/or you trust its source:"),("audit_processors","The following processors make modifications and their use in this recipe should be more closely inspected:")]{if let Some(items)=result[key].as_array(){autopkg_platform::text_println!("    {title}");for item in items{autopkg_platform::text_println!("        {}",py(item));}}}
    autopkg_platform::text_println!();
}
pub fn run(args: &[String]) -> Result<i32, String> {
    let mut prefs = None;
    let mut search = vec![];
    let mut overrides = vec![];
    let mut recipes = vec![];
    let mut plist = false;
    let mut json = false;
    let mut only = None;
    let mut skip = None;
    let mut fail = None;
    let mut list = false;
    let mut recipe_list = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--prefs" => prefs = Some(iter.next().ok_or("--prefs requires a path")?.as_str()),
            "-d" | "--search-dir" => search.push(PathBuf::from(
                iter.next().ok_or("--search-dir requires a path")?,
            )),
            "--override-dir" => overrides.push(PathBuf::from(
                iter.next().ok_or("--override-dir requires a path")?,
            )),
            "-l" | "--recipe-list" => {
                recipe_list = Some(iter.next().ok_or("--recipe-list requires a path")?)
            }
            "-p" | "--plist" => plist = true,
            "-j" | "--json" => json = true,
            "--only-check" => {
                only = Some(iter.next().ok_or("--only-check requires checks")?.as_str())
            }
            "--skip-check" => {
                skip = Some(iter.next().ok_or("--skip-check requires checks")?.as_str())
            }
            "--fail-on" => fail = Some(iter.next().ok_or("--fail-on requires severity")?.as_str()),
            "--list-checks" => list = true,
            "-h" | "--help" => {
                autopkg_platform::text_println!("Usage: russet audit [--json|--plist] [--only-check CHECKS|--skip-check CHECKS] [--fail-on SEVERITY] RECIPE ...");
                return Ok(0);
            }
            "--" => {
                recipes.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unsupported audit option '{flag}'"))
            }
            _ => recipes.push(arg.clone()),
        }
    }
    if list {
        let mut checks = CHECKS;
        checks.sort();
        for (name, _, severity) in checks {
            autopkg_platform::text_println!("{name} ({severity})");
        }
        return Ok(0);
    }
    if plist && json {
        return Err("Only one of --plist and --json may be used.".into());
    }
    if only.is_some() && skip.is_some() {
        return Err("Only one of --only-check and --skip-check may be used.".into());
    }
    if fail.is_some_and(|s| rank(s).is_none()) {
        return Err(format!(
            "Unknown severity '{}'. Valid severities: info, warning, error.",
            fail.unwrap()
        ));
    }
    let names: Vec<_> = only
        .or(skip)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    for name in &names {
        if !CHECKS.iter().any(|c| c.0 == *name) {
            return Err(format!("Unknown check name: {name}"));
        }
    }
    if let Some(path) = recipe_list {
        recipes.extend(super::list_strings(
            &super::read_recipe_list(path)?,
            "recipes",
        )?);
    }
    if recipes.is_empty() {
        return super::options::usage_failure("audit", None, 255);
    }
    let preferences = super::manage::load_preferences(prefs)?;
    if recipes.iter().any(|name| !Path::new(name).is_file()) {
        super::manage::ensure_recipe_map_output(&preferences, plist || json)?;
    }
    let (s, o) = super::manage::recipe_directories(&preferences)?;
    if search.is_empty() {
        search = s
    }
    if overrides.is_empty() {
        overrides = o
    }
    overrides.extend(search);
    let dirs = overrides;
    let mut results = Map::new();
    let mut all = vec![];
    let mut issues = 0;
    for name in &recipes {
        let recipe = match load(Path::new(name), &dirs, &mut HashSet::new()) {
            Ok(recipe) => recipe,
            Err(error) => {
                autopkg_platform::text_eprintln!("No valid recipe found for {name}: {error}");
                continue;
            }
        };
        let mut result = analyze(&recipe);
        if only.is_some() || skip.is_some() {
            result.as_object_mut().unwrap().retain(|key, _| {
                CHECKS
                    .iter()
                    .any(|c| c.1 == key && names.contains(&c.0) == only.is_some())
            });
        }
        if !result.as_object().unwrap().is_empty() {
            issues += 1
        }
        if !plist && !json {
            human(name, &recipe, &result)
        }
        all.push(json!({"recipe":name,"findings":findings(&result)}));
        results.insert(name.clone(), result);
    }
    if plist {
        let output: Plist =
            serde_json::from_value(results.clone().into()).map_err(|e| e.to_string())?;
        output
            .to_writer_xml(std::io::stdout())
            .map_err(|e| e.to_string())?;
    } else if json {
        autopkg_platform::text_println!("{}", {
            #[derive(serde::Serialize)]
            struct Row<'a> {
                recipe: &'a Value,
                findings: &'a Value,
            }
            let rows = all
                .iter()
                .map(|row| Row {
                    recipe: &row["recipe"],
                    findings: &row["findings"],
                })
                .collect::<Vec<_>>();
            serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?
        });
    } else if recipes.len() > 1 {
        autopkg_platform::text_println!("\nSummary:\n    {} recipes audited\n    {issues} recipes with audit issues\n    {} recipes triggered no audit flags",results.len(),results.len()-issues);
    }
    Ok(i32::from(fail.is_some_and(|severity| {
        all.iter().any(|r| {
            r["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| rank(f["severity"].as_str().unwrap()) >= rank(severity))
        })
    })))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_are_static_and_secrets_are_redacted() {
        let recipe = json!({"Identifier":"../bad","Input":{"API_KEY":"DO-NOT-PRINT","TOKEN_URL":"https://endpoint","PASSWORD":"%SECRET%","URL":"http://example","SOURCE":"../outside"},"Process":[{"Processor":"URLDownloader"},{"Processor":"Copier","Arguments":{"source_path":"%SOURCE%"}},{"Processor":"ChocolateyPackager","Arguments":{"installer_checksum_type":"SHA1","id":"../id"}},{"Processor":"CustomPython"}]});
        let result = analyze(&recipe);
        assert!(!result.to_string().contains("DO-NOT-PRINT"));
        assert_eq!(result["sensitive_inputs"].as_array().unwrap().len(), 1);
        assert_eq!(result["weak_hashes"][0]["algorithm"], "sha1");
        assert_eq!(result["path_safety_warnings"].as_array().unwrap().len(), 3);
        assert_eq!(result["non_core_processors"][0], "CustomPython");
        assert_eq!(findings(&result)[0]["severity"], "error");
    }
    #[test]
    fn install_paths_nested_dmg_and_substitution() {
        let recipe = json!({"Input":{"LIST":[{"source_item":"../App","destination_path":"/Applications"}]},"Process":[{"Processor":"InstallFromDMG","Arguments":{"items_to_copy":"%LIST%"}},{"Processor":"Installer","Arguments":{"pkg_path":"%CACHE_DIR%/file.pkg"}},{"Processor":"Copier","Arguments":{"source_path":"archive.DMG/../bad","destination_path":"https://x.dmg/../ignored"}}]});
        let result = paths(&recipe);
        assert_eq!(result.len(), 4);
        assert!(!result.iter().any(|v| v["location"] == "Installer.pkg_path"));
    }
    #[test]
    fn check_option_errors_do_not_touch_files() {
        assert!(run(&["--json".into(), "--plist".into()]).is_err());
        assert!(run(&["--only-check".into(), "future_check".into()]).is_err());
        assert!(run(&["--fail-on".into(), "critical".into()]).is_err());
    }
}
