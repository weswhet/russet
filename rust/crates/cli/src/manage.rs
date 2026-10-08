//! Repository, override, trust, and recipe-map persistence commands.
use autopkg_engine::{
    preferences::Preferences,
    read_recipe,
    trust::{load_verified_recipe, parent_trust_info, TrustPolicy},
};
use plist::{Dictionary, Value};
use serde_json::{json, Value as Json};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

const EXTENSIONS: [&str; 3] = [".recipe", ".recipe.plist", ".recipe.yaml"];
fn home() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or("Home directory is unavailable".into())
}
fn expand(path: &str) -> Result<PathBuf, String> {
    let path = if path == "~" {
        home()?
    } else if let Some(relative) = path.strip_prefix("~/") {
        home()?.join(relative)
    } else {
        PathBuf::from(path)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        env::current_dir().map_err(|e| e.to_string())?.join(path)
    })
}
fn truth(value: Option<&Value>) -> bool {
    match value {
        None => false,
        Some(Value::Boolean(v)) => *v,
        Some(Value::String(v)) => !v.is_empty(),
        Some(Value::Integer(v)) => v.as_signed() != Some(0),
        Some(Value::Array(v)) => !v.is_empty(),
        Some(Value::Dictionary(v)) => !v.is_empty(),
        _ => true,
    }
}
pub fn load_preferences(explicit: Option<&str>) -> Result<Dictionary, String> {
    Ok(Store::load(explicit)?.values)
}
#[cfg(target_os = "macos")]
fn native_keys() -> Result<Vec<String>, String> {
    use core_foundation_sys::{
        array::{CFArrayGetCount, CFArrayGetValueAtIndex},
        base::CFRelease,
        preferences::{kCFPreferencesAnyHost, kCFPreferencesCurrentUser, CFPreferencesCopyKeyList},
        string::{
            kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString,
            CFStringGetLength, CFStringGetMaximumSizeForEncoding,
        },
    };
    unsafe {
        let domain = CFStringCreateWithCString(
            std::ptr::null(),
            c"com.github.autopkg".as_ptr(),
            kCFStringEncodingUTF8,
        );
        let array =
            CFPreferencesCopyKeyList(domain, kCFPreferencesCurrentUser, kCFPreferencesAnyHost);
        CFRelease(domain.cast());
        if array.is_null() {
            return Ok(vec![]);
        }
        let mut result = Vec::new();
        for index in 0..CFArrayGetCount(array) {
            let key = CFArrayGetValueAtIndex(array, index).cast();
            let size =
                CFStringGetMaximumSizeForEncoding(CFStringGetLength(key), kCFStringEncodingUTF8)
                    + 1;
            let mut bytes = vec![0u8; size as usize];
            if CFStringGetCString(key, bytes.as_mut_ptr().cast(), size, kCFStringEncodingUTF8) != 0
            {
                result.push(
                    std::ffi::CStr::from_ptr(bytes.as_ptr().cast())
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        CFRelease(array.cast());
        Ok(result)
    }
}
struct Store {
    values: Dictionary,
    file: Option<Preferences>,
}
impl Store {
    fn load(explicit: Option<&str>) -> Result<Self, String> {
        // Development comparison runs must not consult the user's native domain.
        if let Some(path) = env::var_os("AUTOPKG_RS_PREFERENCES_FILE") {
            let mut file = Preferences::load(Path::new(&path))?;
            if let Some(path) = explicit {
                file.overlay(&expand(path)?)?;
            }
            return Ok(Self {
                values: file.values.clone(),
                file: Some(file),
            });
        }
        let mut values = Dictionary::new();
        #[cfg(target_os = "macos")]
        for key in native_keys()? {
            if let Some(value) = autopkg_platform::preference("com.github.autopkg", &key)? {
                values.insert(key, value);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let default_file = {
            let directory = if cfg!(windows) {
                env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .ok_or("LOCALAPPDATA is unavailable")?
                    .join("Autopkg")
            } else {
                env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or(home()?.join(".config"))
                    .join("Autopkg")
            };
            Preferences::load_directory(&directory)?
        };
        #[cfg(target_os = "macos")]
        let default_file: Option<Preferences> = None;
        if let Some(file) = &default_file {
            values.extend(file.values.clone());
        }
        let file = if let Some(path) = explicit {
            let file = Preferences::load(&expand(path)?)?;
            values.extend(file.values.clone());
            Some(file)
        } else {
            default_file
        };
        Ok(Self { values, file })
    }
    fn save(&mut self, keys: &[&str]) -> Result<(), String> {
        if let Some(file) = &mut self.file {
            file.values = self.values.clone();
            file.save()
        } else {
            #[cfg(target_os = "macos")]
            {
                for key in keys {
                    native_set(key, self.values.get(key).ok_or("Missing preference value")?)?;
                }
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = keys;
                Err("No writable preference file loaded; use --prefs FILE".into())
            }
        }
    }
}
#[cfg(target_os = "macos")]
fn native_set(key: &str, value: &Value) -> Result<(), String> {
    use core_foundation_sys::{
        base::{CFRelease, CFTypeRef},
        data::CFDataCreate,
        preferences::{CFPreferencesAppSynchronize, CFPreferencesSetAppValue},
        propertylist::CFPropertyListCreateWithData,
        string::{kCFStringEncodingUTF8, CFStringCreateWithCString},
    };
    use std::{ffi::CString, ptr};
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }
    let mut bytes = Vec::new();
    value.to_writer_xml(&mut bytes).map_err(|e| e.to_string())?;
    let key = CString::new(key).map_err(|e| e.to_string())?;
    unsafe {
        let domain = Owned(
            CFStringCreateWithCString(
                ptr::null(),
                c"com.github.autopkg".as_ptr(),
                kCFStringEncodingUTF8,
            )
            .cast(),
        );
        let key = Owned(
            CFStringCreateWithCString(ptr::null(), key.as_ptr(), kCFStringEncodingUTF8).cast(),
        );
        let data = Owned(CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize).cast());
        if domain.0.is_null() || key.0.is_null() || data.0.is_null() {
            return Err("Cannot allocate native preference".into());
        }
        let mut error = ptr::null_mut();
        let value = Owned(CFPropertyListCreateWithData(
            ptr::null(),
            data.0.cast(),
            0,
            ptr::null_mut(),
            &mut error,
        ));
        if !error.is_null() {
            CFRelease(error.cast());
        }
        if value.0.is_null() {
            return Err("Cannot decode native preference".into());
        }
        CFPreferencesSetAppValue(key.0.cast(), value.0, domain.0.cast());
        if CFPreferencesAppSynchronize(domain.0.cast()) == 0 {
            return Err("Could not synchronize native preferences".into());
        }
    }
    Ok(())
}
#[derive(Default)]
struct Options {
    prefs: Option<String>,
    search: Vec<String>,
    overrides: Vec<String>,
    args: Vec<String>,
    include_cwd: bool,
    force: bool,
    ignore_deprecation: bool,
    name: Option<String>,
    format: Option<String>,
    verbose: bool,
    help: bool,
}
fn parse(verb: &str, args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--prefs" => o.prefs = Some(iter.next().ok_or("--prefs requires a file")?.clone()),
            "-d" | "--search-dir" if !verb.starts_with("repo-") => o.search.push(
                iter.next()
                    .ok_or("--search-dir requires a directory")?
                    .clone(),
            ),
            "--override-dir" if !verb.starts_with("repo-") => o.overrides.push(
                iter.next()
                    .ok_or("--override-dir requires a directory")?
                    .clone(),
            ),
            "--include-cwd" if verb == "generate-recipe-map" => o.include_cwd = true,
            "-f" | "--force" if verb == "make-override" => o.force = true,
            "-n" | "--name" if verb == "make-override" => {
                o.name = Some(iter.next().ok_or("--name requires a filename")?.clone())
            }
            "--format" if verb == "make-override" => {
                o.format = Some(
                    iter.next()
                        .ok_or("--format requires plist or yaml")?
                        .clone(),
                )
            }
            "--ignore-deprecation" if verb == "make-override" => o.ignore_deprecation = true,
            "-v" | "--verbose" if verb == "verify-trust-info" => o.verbose = true,
            "-l" | "--recipe-list" if verb == "verify-trust-info" => {
                let file = expand(iter.next().ok_or("--recipe-list requires a file")?)?;
                let bytes = fs::read(file).map_err(|e| e.to_string())?;
                if let Ok(value) = Value::from_reader(std::io::Cursor::new(&bytes)) {
                    let recipes = value
                        .as_dictionary()
                        .and_then(|d| d.get("recipes"))
                        .and_then(Value::as_array)
                        .ok_or("Recipe list plist requires recipes array")?;
                    for recipe in recipes {
                        o.args.push(
                            recipe
                                .as_string()
                                .ok_or("Recipe names must be strings")?
                                .into(),
                        );
                    }
                } else {
                    o.args.extend(
                        String::from_utf8(bytes)
                            .map_err(|e| e.to_string())?
                            .lines()
                            .map(str::trim)
                            .filter(|s| !s.is_empty() && !s.starts_with('#'))
                            .map(str::to_owned),
                    );
                }
            }
            "-h" | "--help" => o.help = true,
            "--" => {
                o.args.extend(iter.cloned());
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unsupported {verb} option '{flag}'"))
            }
            _ => o.args.push(arg.clone()),
        }
    }
    Ok(o)
}
fn pref_dirs(values: &Dictionary, key: &str, defaults: &[&str]) -> Result<Vec<String>, String> {
    let paths = match values.get(key) {
        Some(Value::String(v)) if !v.is_empty() => vec![v.clone()],
        Some(Value::Array(v)) if !v.is_empty() => v
            .iter()
            .map(|v| {
                v.as_string()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("{key} must contain strings"))
            })
            .collect::<Result<_, _>>()?,
        _ => defaults.iter().map(|v| (*v).into()).collect(),
    };
    Ok(paths)
}
fn search_dirs(values: &Dictionary) -> Result<Vec<String>, String> {
    pref_dirs(
        values,
        "RECIPE_SEARCH_DIRS",
        &[".", "~/Library/AutoPkg/Recipes", "/Library/AutoPkg/Recipes"],
    )
}
fn override_dirs(values: &Dictionary) -> Result<Vec<String>, String> {
    pref_dirs(
        values,
        "RECIPE_OVERRIDE_DIRS",
        &["~/Library/AutoPkg/RecipeOverrides"],
    )
}
pub fn recipe_directories(values: &Dictionary) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    effective(values, &Options::default())
}
fn repo_root(values: &Dictionary) -> Result<PathBuf, String> {
    expand(
        values
            .get("RECIPE_REPO_DIR")
            .and_then(Value::as_string)
            .unwrap_or("~/Library/AutoPkg/RecipeRepos"),
    )
}
/// Folders whose overrides can't carry trust records: the repo-add clone root
/// and every configured recipe repository. Other search folders, such as `.`,
/// are local and may contain the override folders.
pub fn repository_dirs(values: &Dictionary) -> Result<Vec<PathBuf>, String> {
    let mut dirs = vec![repo_root(values)?];
    for path in repositories(values)?.keys() {
        dirs.push(expand(path)?);
    }
    Ok(dirs)
}
fn effective(values: &Dictionary, o: &Options) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let search = if o.search.is_empty() {
        search_dirs(values)?
    } else {
        o.search.clone()
    };
    let overrides = if o.overrides.is_empty() {
        override_dirs(values)?
    } else {
        o.overrides.clone()
    };
    Ok((
        search.iter().map(|p| expand(p)).collect::<Result<_, _>>()?,
        overrides
            .iter()
            .map(|p| expand(p))
            .collect::<Result<_, _>>()?,
    ))
}
fn name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    EXTENSIONS
        .iter()
        .find_map(|ext| name.strip_suffix(ext).map(str::to_owned))
}
fn identifier(d: &Dictionary) -> Option<&str> {
    d.get("Identifier").and_then(Value::as_string).or_else(|| {
        d.get("Input")
            .and_then(Value::as_dictionary)?
            .get("IDENTIFIER")?
            .as_string()
    })
}
fn scan(directory: &Path, follow: bool) -> Result<Vec<PathBuf>, String> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let root = directory.canonicalize().map_err(|e| e.to_string())?;
    let mut entries = fs::read_dir(directory)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort();
    entries.retain(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'));
    let mut nested = Vec::new();
    for path in &entries {
        if path.is_dir() {
            for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if !path.file_name().unwrap().to_string_lossy().starts_with('.') {
                    nested.push(path);
                }
            }
        }
    }
    nested.sort();
    let mut result = Vec::new();
    for paths in [&entries, &nested] {
        for ext in EXTENSIONS {
            for path in paths {
                if path.is_file()
                    && path.to_string_lossy().ends_with(ext)
                    && (follow
                        || path
                            .canonicalize()
                            .map(|p| p.starts_with(&root))
                            .unwrap_or(false))
                {
                    result.push(path.clone());
                }
            }
        }
    }
    Ok(result)
}
fn locate(request: &str, dirs: &[PathBuf], follow: bool) -> Result<PathBuf, String> {
    let direct = expand(request)?;
    if direct.is_file() {
        return Ok(direct);
    }
    let requested = name(Path::new(request)).unwrap_or_else(|| request.into());
    for directory in dirs {
        for path in scan(directory, follow)? {
            if name(&path).as_deref() == Some(&requested) {
                return Ok(path);
            }
        }
    }
    for directory in dirs {
        for path in scan(directory, follow)? {
            if read_recipe(&path).ok().as_ref().and_then(identifier) == Some(request) {
                return Ok(path);
            }
        }
    }
    Err(format!("No valid recipe found for {request}"))
}
fn map_data(values: &Dictionary, include_cwd: bool) -> Result<Json, String> {
    let mut data = json!({"schema_version":1,"identifiers":{},"shortnames":{},"overrides":{},"overrides-identifiers":{}});
    for (directories, id_key, name_key, follow) in [
        (search_dirs(values)?, "identifiers", "shortnames", false),
        (
            override_dirs(values)?,
            "overrides-identifiers",
            "overrides",
            true,
        ),
    ] {
        for directory in directories {
            if directory == "." && !include_cwd {
                continue;
            }
            for path in scan(&expand(&directory)?, follow)? {
                let path_text = path.to_string_lossy().into_owned();
                let name = name(&path).unwrap();
                data[name_key]
                    .as_object_mut()
                    .unwrap()
                    .entry(name)
                    .or_insert(Json::String(path_text.clone()));
                if let Some(id) = read_recipe(&path).ok().as_ref().and_then(identifier) {
                    data[id_key]
                        .as_object_mut()
                        .unwrap()
                        .entry(id)
                        .or_insert(Json::String(path_text));
                }
            }
        }
    }
    Ok(data)
}
fn map_path(values: &Dictionary) -> Result<PathBuf, String> {
    map_path_output(values, false)
}
fn map_path_output(values: &Dictionary, stderr: bool) -> Result<PathBuf, String> {
    #[cfg(unix)]
    let root = unsafe { libc::geteuid() } == 0;
    #[cfg(not(unix))]
    let root = false;
    let environment = env::var("AUTOPKG_RECIPE_MAP_PATH")
        .ok()
        .filter(|s| !s.is_empty());
    if root && environment.is_some() {
        autopkg_platform::text_eprintln!(
            "SECURITY WARNING: autopkg is running as root and AUTOPKG_RECIPE_MAP_PATH was ignored."
        );
    }
    let target = environment
        .filter(|_| !root)
        .or_else(|| {
            values
                .get("RECIPE_MAP_PATH")
                .and_then(Value::as_string)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "~/Library/AutoPkg/recipe_map.json".into());
    if env::var("AUTOPKG_RECIPE_MAP_PATH")
        .ok()
        .is_some_and(|s| !s.is_empty())
        && !root
    {
        map_log(&format!("Recipe map path overridden via AUTOPKG_RECIPE_MAP_PATH environment variable: {target}"), stderr);
    } else if values
        .get("RECIPE_MAP_PATH")
        .and_then(Value::as_string)
        .is_some_and(|s| !s.is_empty())
    {
        map_log(
            &format!("Recipe map path overridden via RECIPE_MAP_PATH preference: {target}"),
            stderr,
        );
    }
    #[cfg(unix)]
    if root && (target == "~" || target.starts_with("~/")) {
        unsafe {
            let account = libc::getpwuid(0);
            if account.is_null() {
                return Err("Cannot find root home directory".into());
            }
            let home = std::ffi::CStr::from_ptr((*account).pw_dir).to_string_lossy();
            return Ok(PathBuf::from(home.as_ref()).join(target.strip_prefix("~/").unwrap_or("")));
        }
    }
    expand(&target)
}
fn atomic_write(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let parent = path.parent().ok_or("Destination has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    if replace {
        file.persist(path).map_err(|e| e.to_string())?;
    } else {
        file.persist_noclobber(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn map_log(message: &str, stderr: bool) {
    if stderr {
        autopkg_platform::text_eprintln!("{message}")
    } else {
        autopkg_platform::text_println!("{message}")
    }
}
pub fn ensure_recipe_map(values: &Dictionary) -> Result<(), String> {
    ensure_recipe_map_output(values, false)
}
pub fn ensure_recipe_map_output(values: &Dictionary, stderr: bool) -> Result<(), String> {
    if env::var_os("AUTOPKG_DISABLE_RECIPE_MAP").is_some_and(|v| !v.is_empty())
        || truth(values.get("DISABLE_RECIPE_MAP"))
    {
        return Ok(());
    }
    let path = map_path_output(values, stderr)?;
    if path.exists() {
        return Ok(());
    }
    map_log("Recipe map not found; generating it on demand. Run `autopkg generate-recipe-map` explicitly (e.g. in CI) to avoid paying this cost on every fresh run.",stderr);
    let mut bytes =
        serde_json::to_vec_pretty(&map_data(values, false)?).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    atomic_write(&map_path_output(values, stderr)?, &bytes, true)
}
fn refresh_map(values: &Dictionary) -> Result<(), String> {
    if env::var_os("AUTOPKG_DISABLE_RECIPE_MAP").is_some_and(|v| !v.is_empty())
        || truth(values.get("DISABLE_RECIPE_MAP"))
    {
        return Ok(());
    }
    let path = map_path(values)?;
    let mut bytes =
        serde_json::to_vec_pretty(&map_data(values, false)?).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    atomic_write(&path, &bytes, true)
}
fn write_map(values: &Dictionary, include_cwd: bool) -> Result<(), String> {
    if env::var_os("AUTOPKG_DISABLE_RECIPE_MAP").is_some_and(|v| !v.is_empty())
        || truth(values.get("DISABLE_RECIPE_MAP"))
    {
        return Ok(());
    }
    let _ = map_path(values)?;
    let data = map_data(values, include_cwd)?;
    let path = map_path(values)?;
    let mut bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if let Err(error) = atomic_write(&path, &bytes, true) {
        autopkg_platform::text_eprintln!(
            "WARNING: Could not write recipe map {}: {error}",
            path.display()
        );
        return Ok(());
    }
    autopkg_platform::text_println!("Recipe map written to {}:\n  identifiers:           {}\n  shortnames:            {}\n  overrides:             {}\n  overrides-identifiers: {}",path.display(),data["identifiers"].as_object().unwrap().len(),data["shortnames"].as_object().unwrap().len(),data["overrides"].as_object().unwrap().len(),data["overrides-identifiers"].as_object().unwrap().len());
    Ok(())
}
fn write_recipe(path: &Path, data: Dictionary, replace: bool) -> Result<(), String> {
    let bytes = if path.to_string_lossy().ends_with(".recipe.yaml") {
        autopkg_engine::to_recipe_yaml(&Value::Dictionary(data))?.into_bytes()
    } else {
        let mut bytes = Vec::new();
        Value::Dictionary(data)
            .to_writer_xml(&mut bytes)
            .map_err(|e| e.to_string())?;
        bytes
    };
    atomic_write(path, &bytes, replace)
}
fn make_override(store: &Store, o: &Options) -> Result<i32, String> {
    if o.args.len() != 1 {
        autopkg_platform::text_eprintln!("Need exactly one recipe to override!");
        return Ok(255);
    }
    if expand(&o.args[0])?.is_file() {
        return Err("make-override doesn't work with absolute recipe paths".into());
    }
    ensure_recipe_map(&store.values)?;
    let (search, overrides) = effective(&store.values, o)?;
    let path = locate(&o.args[0], &search, false)?;
    let raw = read_recipe(&path)?;
    let id = identifier(&raw).ok_or("Parent recipe is missing an Identifier")?;
    let recipe = autopkg_engine::load_recipe(&path, &search)?;
    if !o.ignore_deprecation
        && recipe
            .process
            .iter()
            .any(|p| p.processor == "DeprecationWarning")
    {
        return Err("Recipe or one of its parents is deprecated. Use --ignore-deprecation to make an override regardless.".into());
    }
    let name = o
        .name
        .clone()
        .or_else(|| name(&path))
        .ok_or("Cannot determine override name")?;
    if name.is_empty() || name.contains(['/', '\\']) || matches!(name.as_str(), "." | "..") {
        return Err("Override name must be a filename without directory components".into());
    }
    let format = o
        .format
        .as_deref()
        .or_else(|| {
            store
                .values
                .get("RECIPE_OVERRIDE_FORMAT")
                .and_then(Value::as_string)
        })
        .unwrap_or("plist")
        .to_lowercase();
    if !matches!(format.as_str(), "plist" | "yaml") {
        return Err("Override format must be plist or yaml".into());
    }
    let mut input = recipe.input;
    input.remove("IDENTIFIER");
    let data = Dictionary::from_iter([
        (
            "Identifier",
            Value::String(format!(
                "local.{}",
                name.split('.').rev().collect::<Vec<_>>().join(".")
            )),
        ),
        ("Input", Value::Dictionary(input)),
        ("ParentRecipe", Value::String(id.into())),
        (
            "ParentRecipeTrustInfo",
            Value::Dictionary(parent_trust_info(&path, &search)?),
        ),
    ]);
    let output = overrides
        .first()
        .ok_or("No override directory configured")?
        .join(format!(
            "{name}.recipe{}",
            if format == "yaml" { ".yaml" } else { "" }
        ));
    if output.exists() && !o.force {
        return Err(format!("A recipe override already exists at {}, will not overwrite it. Use --force to overwrite anyway.",output.display()));
    }
    write_recipe(&output, data, o.force)?;
    autopkg_platform::text_println!("Override file saved to {}", output.display());
    // Build only preference-scoped buckets. A transient --override-dir must
    // never become visible to unrelated invocations through the persistent map.
    let configured = override_dirs(&store.values)?
        .iter()
        .map(|p| expand(p))
        .collect::<Result<Vec<_>, _>>()?;
    if configured.iter().any(|p| output.starts_with(p)) {
        let path = map_path(&store.values)?;
        let mut bytes = serde_json::to_vec_pretty(&map_data(&store.values, false)?)
            .map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        atomic_write(&path, &bytes, true)?;
    }
    Ok(0)
}
fn trust(store: &Store, o: &Options, verify: bool) -> Result<i32, String> {
    if o.args.is_empty() {
        return super::options::usage_failure(
            if verify {
                "verify-trust-info"
            } else {
                "update-trust-info"
            },
            Some("Need at least one recipe name or path!"),
            255,
        );
    }
    let (search, overrides) = effective(&store.values, o)?;
    let mut dirs = overrides.clone();
    dirs.extend(search.clone());
    let repositories = repository_dirs(&store.values)?;
    let policy = TrustPolicy {
        override_dirs: overrides.clone(),
        repository_dirs: repositories.clone(),
    };
    let mut status = 0;
    if verify && o.args.iter().any(|name| !Path::new(name).is_file()) {
        ensure_recipe_map(&store.values)?;
    }
    for requested in &o.args {
        let operation = (|| {
            let path = locate(requested, &dirs, true)?;
            if verify {
                load_verified_recipe(&path, &dirs, &policy)?;
                autopkg_platform::text_println!("{requested}: OK");
                return Ok(());
            }
            if read_recipe(&path)?.get("ParentRecipe").is_none() {
                autopkg_platform::text_eprintln!(
                    "{requested} is not a recipe override and has no parent recipe.\nPath: {}",
                    path.display()
                );
                return Ok(());
            }
            let resolved = path.canonicalize().map_err(|e| e.to_string())?;
            let inside = |roots: &[PathBuf]| {
                roots
                    .iter()
                    .filter_map(|p| p.canonicalize().ok())
                    .any(|p| resolved.starts_with(p))
            };
            if !inside(&overrides) || inside(&repositories) {
                return Err(
                    "Trust records must come from a local override outside recipe repositories"
                        .into(),
                );
            }
            let mut data = read_recipe(&path)?;
            let parent = data
                .get("ParentRecipe")
                .and_then(Value::as_string)
                .ok_or("Recipe is not an override and has no parent recipe")?;
            let mut parent_dirs = search.clone();
            parent_dirs.push(path.parent().unwrap().to_owned());
            let parent = locate(parent, &parent_dirs, false)?;
            data.insert(
                "ParentRecipeTrustInfo".into(),
                parent_trust_info(&parent, &parent_dirs)?.into(),
            );
            write_recipe(&path, data, true)?;
            autopkg_platform::text_println!("Wrote updated {}", path.display());
            Ok::<(), String>(())
        })();
        if let Err(error) = operation {
            status = 1;
            autopkg_platform::text_eprintln!("{requested}: FAILED");
            if o.verbose || !verify {
                autopkg_platform::text_eprintln!("    {error}");
            }
        }
    }
    Ok(status)
}
fn windows_local_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with(r"\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}
fn expand_repo_url(value: &str) -> String {
    if windows_local_path(value) {
        return value.into();
    }
    let value = value.trim_end_matches('/');
    if value.starts_with(['/', '~']) || value.contains("://") {
        return value.into();
    }
    if let Some(colon) = value.find(':') {
        if value.find('/').is_none_or(|slash| colon < slash) {
            return format!("ssh://{}/{}", &value[..colon], &value[colon + 1..]);
        }
    }
    if value.contains('/') {
        format!("https://github.com/{value}")
    } else {
        format!("https://github.com/autopkg/{value}")
    }
}
fn repo_name(url: &str) -> String {
    // Local Windows paths are Git filesystem arguments, not URL authorities.
    // Strip the drive colon and flatten separators before constructing a child name.
    let local;
    let url = if windows_local_path(url) {
        local = url.replace(':', "").replace('\\', "/");
        &local
    } else {
        url
    };
    let (authority, path) = if let Some((_, tail)) = url.split_once("://") {
        tail.split_once('/')
            .map(|(host, path)| (host, format!("/{path}")))
            .unwrap_or((tail, String::new()))
    } else {
        ("", url.into())
    };
    let host = authority
        .rsplit('@')
        .next()
        .unwrap()
        .split(':')
        .next()
        .unwrap();
    let domain = host.split('.').rev().collect::<Vec<_>>().join(".");
    let path = if let Some(dot) = path.rfind('.') {
        if path.rfind('/').is_none_or(|slash| dot > slash) {
            &path[..dot]
        } else {
            &path
        }
    } else {
        &path
    };
    format!("{domain}{}", path.replace('/', "."))
}
fn git(values: &Dictionary, dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let program = values
        .get("GIT_PATH")
        .and_then(Value::as_string)
        .unwrap_or("git");
    let mut command = Command::new(program);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    command.args(args);
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim_end().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"))
}
fn repositories(values: &Dictionary) -> Result<Dictionary, String> {
    match values.get("RECIPE_REPOS") {
        None => Ok(Dictionary::new()),
        Some(v) => v
            .as_dictionary()
            .cloned()
            .ok_or("RECIPE_REPOS must be a dictionary".into()),
    }
}
fn repo_match(repos: &Dictionary, requested: &str) -> Option<String> {
    let expanded = expand_repo_url(requested);
    if expanded.contains("://") {
        repos
            .iter()
            .find(|(_, v)| {
                v.as_dictionary()
                    .and_then(|d| d.get("URL"))
                    .and_then(Value::as_string)
                    == Some(&expanded)
            })
            .map(|(p, _)| p.clone())
    } else {
        let path = expand(&expanded).ok()?.to_string_lossy().into_owned();
        repos.contains_key(&path).then_some(path)
    }
}
fn repo_list(values: &Dictionary) -> Result<(), String> {
    let repos = repositories(values)?;
    if repos.is_empty() {
        autopkg_platform::text_println!("No recipe repos.");
    } else {
        let mut entries: Vec<_> = repos.iter().collect();
        entries.sort_by_key(|(path, _)| *path);
        for (path, value) in entries {
            let url = value
                .as_dictionary()
                .and_then(|d| d.get("URL"))
                .and_then(Value::as_string)
                .ok_or("Repository entry has no URL")?;
            autopkg_platform::text_println!("{path} ({url})");
        }
        autopkg_platform::text_println!();
    }
    Ok(())
}
fn repo_add(store: &mut Store, o: &Options) -> Result<i32, String> {
    if o.args.is_empty() {
        autopkg_platform::text_eprintln!("Need at least one recipe repo URL!");
        return Ok(255);
    }
    let mut repos = repositories(&store.values)?;
    let mut search = search_dirs(&store.values)?;
    let root = repo_root(&store.values)?;
    let mut changed = false;
    for requested in &o.args {
        if requested.contains("file://") {
            autopkg_platform::text_eprintln!(
                "AutoPkg does not handle file:// URIs; add to your local Recipes folder instead."
            );
            continue;
        }
        let url = expand_repo_url(requested);
        let path = root.join(repo_name(&url));
        let operation = (|| {
            if path.exists() {
                if !path.join(".git").is_dir() {
                    return Err(format!("{} exists and is not a git repo!", path.display()));
                }
                autopkg_platform::text_println!("Attempting git pull for {}...", path.display());
                autopkg_platform::text_println!("{}", git(&store.values, Some(&path), &["pull"])?);
            } else {
                fs::create_dir_all(&root).map_err(|e| e.to_string())?;
                autopkg_platform::text_println!("Attempting git clone for {url}...");
                autopkg_platform::text_println!(
                    "{}",
                    git(
                        &store.values,
                        None,
                        &["clone", "--", &url, &path.to_string_lossy()],
                    )?
                );
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = operation {
            autopkg_platform::text_eprintln!("{error}");
            continue;
        }
        let path = path.to_string_lossy().into_owned();
        if !search.contains(&path) {
            autopkg_platform::text_println!("Adding {path} to RECIPE_SEARCH_DIRS...");
            search.push(path.clone());
        }
        repos.insert(path, Dictionary::from_iter([("URL", url)]).into());
        changed = true;
    }
    store.values.insert("RECIPE_REPOS".into(), repos.into());
    store.values.insert(
        "RECIPE_SEARCH_DIRS".into(),
        search
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>()
            .into(),
    );
    store.save(&["RECIPE_REPOS", "RECIPE_SEARCH_DIRS"])?;
    if changed {
        refresh_map(&store.values)?;
    }
    autopkg_platform::text_println!("Updated search path:");
    for path in search {
        autopkg_platform::text_println!("  '{path}'");
    }
    Ok(0)
}
// Python 3.11 shutil.rmtree stops at a read-only Windows file. Rust's
// remove_dir_all clears that attribute, which would change failure behavior and
// delete more of the checkout than the reference operation authorized.
#[cfg(windows)]
fn remove_repository(path: &Path) -> Result<(), String> {
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|e| e.to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            remove_repository(&child)?;
        } else {
            if metadata.permissions().readonly() {
                let text = child.to_string_lossy();
                let quote = if text.contains('\'') && !text.contains('"') {
                    '"'
                } else {
                    '\''
                };
                let escaped = text
                    .replace('\\', "\\\\")
                    .replace(quote, &format!("\\{quote}"));
                return Err(format!(
                    "[WinError 5] Access is denied: {quote}{escaped}{quote}"
                ));
            }
            fs::remove_file(&child).map_err(|e| e.to_string())?;
        }
    }
    fs::remove_dir(path).map_err(|e| e.to_string())
}
#[cfg(not(windows))]
fn remove_repository(path: &Path) -> Result<(), String> {
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}
fn repo_delete(store: &mut Store, o: &Options) -> Result<i32, String> {
    if o.args.is_empty() {
        autopkg_platform::text_eprintln!("Need at least one recipe repo path or URL!");
        return Ok(255);
    }
    let mut repos = repositories(&store.values)?;
    let mut search = search_dirs(&store.values)?;
    let mut changed = false;
    for requested in &o.args {
        let Some(path) = repo_match(&repos, requested) else {
            autopkg_platform::text_eprintln!(
                "ERROR: Can't find an installed repo for {}",
                expand_repo_url(requested)
            );
            continue;
        };
        let destination = Path::new(&path);
        if destination.parent().is_none() || destination == home()? {
            return Err(
                "Refusing to remove a repository configured at a filesystem root or home directory"
                    .into(),
            );
        }
        autopkg_platform::text_println!("Removing repo at {path}...");
        repos.remove(&path);
        search.retain(|p| p != &path);
        changed = true;
        let deletion = if fs::symlink_metadata(destination)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            Err("Cannot remove a repository through a symlink".to_string())
        } else {
            remove_repository(destination)
        };
        if let Err(error) = deletion {
            autopkg_platform::text_eprintln!("ERROR: Could not remove {path}: {error}. The repo has been deconfigured; remove the directory manually if desired.");
        }
    }
    store.values.insert("RECIPE_REPOS".into(), repos.into());
    store.values.insert(
        "RECIPE_SEARCH_DIRS".into(),
        search
            .into_iter()
            .map(Value::String)
            .collect::<Vec<_>>()
            .into(),
    );
    store.save(&["RECIPE_REPOS", "RECIPE_SEARCH_DIRS"])?;
    if changed {
        refresh_map(&store.values)?;
    }
    Ok(0)
}
fn migrate(values: &Dictionary, path: &Path) -> Result<bool, String> {
    if git(values, Some(path), &["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_default()
        .trim()
        != "master"
    {
        return Ok(false);
    }
    git(values, Some(path), &["fetch", "origin", "--prune"])?;
    let main = git(
        values,
        Some(path),
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/main",
        ],
    )
    .is_ok();
    let master = git(
        values,
        Some(path),
        &[
            "show-ref",
            "--verify",
            "--quiet",
            "refs/remotes/origin/master",
        ],
    )
    .is_ok();
    if main && !master {
        git(values, Some(path), &["branch", "-m", "master", "main"])?;
        git(
            values,
            Some(path),
            &["branch", "--set-upstream-to=origin/main", "main"],
        )?;
        git(
            values,
            Some(path),
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        )?;
        Ok(true)
    } else {
        Ok(false)
    }
}
fn repo_update(store: &Store, o: &Options) -> Result<i32, String> {
    if o.args.is_empty() {
        autopkg_platform::text_eprintln!("Need at least one recipe repo path or URL!");
        return Ok(255);
    }
    let repos = repositories(&store.values)?;
    let paths: Vec<String> = if o.args.iter().any(|a| a == "all") {
        repos.keys().cloned().collect()
    } else {
        o.args
            .iter()
            .filter_map(|name| {
                let path = repo_match(&repos, name);
                if path.is_none() {
                    autopkg_platform::text_eprintln!(
                        "ERROR: Can't find an installed repo for {}",
                        expand_repo_url(name)
                    );
                }
                path
            })
            .collect()
    };
    let mut changed = false;
    for path in paths {
        let path = expand(&path)?;
        autopkg_platform::text_println!("Attempting git pull for {}...", path.display());
        let operation = (|| {
            let migrated = migrate(&store.values, &path)?;
            let before = git(&store.values, Some(&path), &["rev-parse", "HEAD"]).ok();
            autopkg_platform::text_println!("{}", git(&store.values, Some(&path), &["pull"])?);
            let after = git(&store.values, Some(&path), &["rev-parse", "HEAD"]).ok();
            Ok::<bool, String>(migrated || before.is_none() || after.is_none() || before != after)
        })();
        match operation {
            Ok(value) => changed |= value,
            Err(error) => autopkg_platform::text_eprintln!("{error}"),
        }
    }
    if changed {
        refresh_map(&store.values)?;
    }
    Ok(0)
}
pub fn new_recipe(args: &[String]) -> Result<i32, String> {
    let mut identifier = None;
    let mut parent = None;
    let mut format = "plist".to_string();
    let mut prefs = None;
    let mut filename = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-i" | "--identifier" => {
                identifier = Some(iter.next().ok_or("--identifier requires a value")?.clone())
            }
            "-p" | "--parent-identifier" => {
                parent = Some(
                    iter.next()
                        .ok_or("--parent-identifier requires a value")?
                        .clone(),
                )
            }
            "--format" => format = iter.next().ok_or("--format requires a value")?.clone(),
            "--prefs" => prefs = Some(iter.next().ok_or("--prefs requires a path")?.clone()),
            "-h" | "--help" => {
                autopkg_platform::text_println!("Usage: russet new-recipe [--identifier ID] [--parent-identifier ID] [--format plist|yaml] recipe_pathname");
                return Ok(0);
            }
            flag if flag.starts_with('-') => {
                return Err(format!("Unsupported new-recipe option '{flag}'"))
            }
            path => {
                if filename.replace(path.to_string()).is_some() {
                    return super::options::usage_failure(
                        "new-recipe",
                        Some("Must specify exactly one recipe pathname!"),
                        255,
                    );
                }
            }
        }
    }
    let Some(filename) = filename else {
        return super::options::usage_failure(
            "new-recipe",
            Some("Must specify exactly one recipe pathname!"),
            255,
        );
    };
    let path = Path::new(&filename);
    let name = path
        .file_name()
        .ok_or("Recipe path must name a file")?
        .to_string_lossy()
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();
    let yaml = format == "yaml" || filename.ends_with(".recipe.yaml");
    let mut data = Dictionary::from_iter([
        ("Description", Value::from("Recipe description")),
        (
            "Identifier",
            Value::from(identifier.unwrap_or_else(|| format!("local.{name}"))),
        ),
        (
            "Input",
            Value::Dictionary(Dictionary::from_iter([("NAME", Value::from(name))])),
        ),
        (
            "MinimumVersion",
            Value::from(if yaml { "2.3" } else { "1.0" }),
        ),
        (
            "Process",
            Value::Array(vec![Value::Dictionary(Dictionary::from_iter([
                (
                    "Arguments",
                    Value::Dictionary(Dictionary::from_iter([
                        ("Argument1", Value::from("Value1")),
                        ("Argument2", Value::from("Value2")),
                    ])),
                ),
                ("Processor", Value::from("ProcessorName")),
            ]))]),
        ),
    ]);
    if let Some(parent) = parent {
        data.insert("ParentRecipe".into(), parent.into());
    }
    let value = Value::Dictionary(data);
    if yaml {
        fs::write(path, autopkg_engine::to_recipe_yaml(&value)?)
            .map_err(|e| format!("Failed to write recipe: {e}"))?;
    } else {
        value
            .to_file_xml(path)
            .map_err(|e| format!("Failed to write recipe: {e}"))?;
    }
    autopkg_platform::text_println!("Saved new recipe to {filename}");
    let values = load_preferences(prefs.as_deref())?;
    let target = path.canonicalize().map_err(|e| e.to_string())?;
    let mut indexed = false;
    for root in search_dirs(&values)? {
        for candidate in scan(&expand(&root)?, false)? {
            if candidate.canonicalize().ok().as_ref() == Some(&target) {
                indexed = true;
            }
        }
    }
    if indexed {
        write_map(&values, false)?;
    }
    Ok(0)
}

pub fn run(verb: &str, args: &[String]) -> Result<i32, String> {
    let verb = if verb == "list-repos" {
        "repo-list"
    } else {
        verb
    };
    let options = parse(verb, args)?;
    if options.help {
        autopkg_platform::text_println!(
            "Usage: russet {verb} [--prefs FILE] [options] [arguments]"
        );
        return Ok(0);
    }
    if verb == "generate-recipe-map"
        && (!options.search.is_empty() || !options.overrides.is_empty())
    {
        return Err("generate-recipe-map writes to the persistent on-disk cache, so transient --search-dir / --override-dir flags are not accepted".into());
    }
    let mut store = Store::load(options.prefs.as_deref())?;
    match verb {
        "repo-list" => {
            repo_list(&store.values)?;
            Ok(0)
        }
        "repo-add" => repo_add(&mut store, &options),
        "repo-delete" => repo_delete(&mut store, &options),
        "repo-update" => repo_update(&store, &options),
        "generate-recipe-map" => {
            write_map(&store.values, options.include_cwd)?;
            Ok(0)
        }
        "make-override" => make_override(&store, &options),
        "update-trust-info" => trust(&store, &options, false),
        "verify-trust-info" => trust(&store, &options, true),
        _ => Err(format!(
            "Command '{verb}' is not implemented in this development build"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recipe(path: &Path, id: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::String(id.into())),
            ("Input", Dictionary::from_iter([("NAME", "App")]).into()),
            ("Process", Value::Array(vec![])),
        ]))
        .to_file_xml(path)
        .unwrap();
    }
    fn store(root: &Path) -> Store {
        let search = root.join("recipes");
        let overrides = root.join("overrides");
        fs::create_dir_all(&search).unwrap();
        fs::create_dir_all(&overrides).unwrap();
        let values = Dictionary::from_iter([
            (
                "RECIPE_SEARCH_DIRS",
                Value::Array(vec![search.to_string_lossy().into_owned().into()]),
            ),
            (
                "RECIPE_OVERRIDE_DIRS",
                Value::Array(vec![overrides.to_string_lossy().into_owned().into()]),
            ),
            (
                "RECIPE_REPO_DIR",
                root.join("repos").to_string_lossy().into_owned().into(),
            ),
            (
                "RECIPE_MAP_PATH",
                root.join("recipe_map.json")
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            ),
        ]);
        let file = Preferences {
            values: values.clone(),
            path: root.join("prefs.plist"),
            format: autopkg_engine::preferences::Format::Plist,
        };
        file.save().unwrap();
        Store {
            values,
            file: Some(file),
        }
    }
    #[test]
    fn yaml_recipe_writer_preserves_environment_types() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("Types.recipe.yaml");
        let data = Dictionary::from_iter([
            ("Identifier", Value::from("org.types")),
            (
                "Input",
                Value::Dictionary(Dictionary::from_iter([
                    (
                        "DATE",
                        Value::Date(plist::Date::from_xml_format("2024-01-02T03:04:05Z").unwrap()),
                    ),
                    ("DATA", Value::Data(vec![0, 255, 1])),
                    ("REAL", Value::Real(1.0)),
                    ("NULL", Value::Null),
                    ("STRING", Value::from("true")),
                ])),
            ),
        ]);
        write_recipe(&path, data.clone(), false).unwrap();
        assert_eq!(autopkg_engine::read_recipe(&path).unwrap(), data);
    }
    #[test]
    fn new_recipe_templates_and_yaml_extension() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        for (filename, format, version) in [
            ("Example.download.recipe", "plist", "1.0"),
            ("Example.recipe.yaml", "plist", "2.3"),
            ("Explicit.recipe", "yaml", "2.3"),
        ] {
            let target = temp.path().join(filename);
            let args = vec![
                "--prefs".into(),
                store
                    .file
                    .as_ref()
                    .unwrap()
                    .path
                    .to_string_lossy()
                    .into_owned(),
                "--format".into(),
                format.into(),
                "-i".into(),
                "org.example".into(),
                "-p".into(),
                "org.parent".into(),
                target.to_string_lossy().into_owned(),
            ];
            assert_eq!(new_recipe(&args).unwrap(), 0);
            let bytes = fs::read(&target).unwrap();
            let value = if version == "2.3" {
                serde_yaml::from_slice::<Value>(&bytes).unwrap()
            } else {
                Value::from_reader(std::io::Cursor::new(bytes)).unwrap()
            };
            let data = value.as_dictionary().unwrap();
            assert_eq!(data["MinimumVersion"].as_string(), Some(version));
            assert_eq!(data["Identifier"].as_string(), Some("org.example"));
            assert_eq!(data["ParentRecipe"].as_string(), Some("org.parent"));
            assert_eq!(
                data["Input"].as_dictionary().unwrap()["NAME"].as_string(),
                Some(if filename.starts_with("Explicit") {
                    "Explicit"
                } else {
                    "Example"
                })
            );
        }
    }
    #[test]
    fn map_preserves_schema_first_wins_and_scan_depth() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let root = temp.path().join("recipes");
        recipe(&root.join("App.recipe"), "org.app");
        recipe(&root.join("nested/App.recipe.plist"), "org.second");
        recipe(&root.join("nested/deep/Hidden.recipe"), "org.hidden");
        let data = map_data(&store.values, false).unwrap();
        assert_eq!(data["schema_version"], 1);
        assert_eq!(data["identifiers"].as_object().unwrap().len(), 2);
        assert_eq!(
            Path::new(data["shortnames"]["App"].as_str().unwrap()),
            root.join("App.recipe")
        );
        let path = temp.path().join("isolated-map.json");
        atomic_write(&path, &serde_json::to_vec(&data).unwrap(), true).unwrap();
        assert_eq!(
            serde_json::from_slice::<Json>(&fs::read(&path).unwrap()).unwrap(),
            data
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn map_confines_repository_symlinks_but_accepts_override_links() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path());
        let outside = temp.path().join("outside.recipe");
        recipe(&outside, "org.outside");
        std::os::unix::fs::symlink(&outside, temp.path().join("recipes/Link.recipe")).unwrap();
        std::os::unix::fs::symlink(&outside, temp.path().join("overrides/Link.recipe")).unwrap();
        let data = map_data(&store.values, false).unwrap();
        assert!(data["identifiers"].as_object().unwrap().is_empty());
        assert_eq!(data["overrides-identifiers"].as_object().unwrap().len(), 1);
    }
    #[test]
    fn override_and_trust_round_trip_detects_parent_change() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = store(temp.path());
        store
            .values
            .insert("DISABLE_RECIPE_MAP".into(), true.into());
        let parent = temp.path().join("recipes/App.download.recipe");
        recipe(&parent, "org.app.download");
        let options = Options {
            args: vec!["App.download".into()],
            ..Default::default()
        };
        make_override(&store, &options).unwrap();
        let child = temp.path().join("overrides/App.download.recipe");
        let data = read_recipe(&child).unwrap();
        assert_eq!(identifier(&data), Some("local.download.App"));
        assert!(make_override(&store, &options).is_err());
        assert_eq!(trust(&store, &options, true).unwrap(), 0);
        let mut changed = read_recipe(&parent).unwrap();
        changed.insert("Description".into(), "changed".into());
        Value::Dictionary(changed).to_file_xml(&parent).unwrap();
        assert_eq!(trust(&store, &options, true).unwrap(), 1);
        assert_eq!(trust(&store, &options, false).unwrap(), 0);
        assert_eq!(trust(&store, &options, true).unwrap(), 0);
        let yaml = Options {
            name: Some("Yaml.download".into()),
            format: Some("yaml".into()),
            ..options
        };
        make_override(&store, &yaml).unwrap();
        assert!(read_recipe(&temp.path().join("overrides/Yaml.download.recipe.yaml")).is_ok());
    }
    #[test]
    fn repository_url_expansion_and_names_match_reference() {
        assert_eq!(
            expand_repo_url("recipes"),
            "https://github.com/autopkg/recipes"
        );
        assert_eq!(
            expand_repo_url("user/recipes/"),
            "https://github.com/user/recipes"
        );
        assert_eq!(
            expand_repo_url("git@github.com:user/recipes.git"),
            "ssh://git@github.com/user/recipes.git"
        );
        for path in [
            r"C:\Users\fixture\recipes.git",
            "C:/Users/fixture/recipes.git",
            r"\\server\share\recipes.git",
        ] {
            assert_eq!(expand_repo_url(path), path);
            let name = repo_name(path);
            assert!(!name.contains([':', '/', '\\']));
        }
        assert_eq!(
            repo_name(r"C:\Users\fixture\recipes.git"),
            "C.Users.fixture.recipes"
        );
        assert_eq!(
            repo_name("ssh://git@github.com:22/user/recipes.git"),
            "com.github.user.recipes"
        );
        assert!(parse("generate-recipe-map", &["--unknown".into()]).is_err());
        assert!(parse("make-override", &["--pull".into()]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn repository_removal_preserves_readonly_file_like_python() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        let object = repo.join("object");
        fs::write(&object, b"keep").unwrap();
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&object, permissions).unwrap();
        let error = remove_repository(&repo).unwrap_err();
        assert!(error.starts_with("[WinError 5] Access is denied:"));
        assert_eq!(fs::read(&object).unwrap(), b"keep");
    }
    #[test]
    fn local_repository_add_list_and_delete_preserve_preferences() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = store(temp.path());
        store
            .values
            .insert("DISABLE_RECIPE_MAP".into(), true.into());
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        git(
            &store.values,
            Some(&source),
            &["init", "--initial-branch=main"],
        )
        .unwrap();
        recipe(&source.join("App.recipe"), "org.app");
        git(&store.values, Some(&source), &["add", "App.recipe"]).unwrap();
        git(
            &store.values,
            Some(&source),
            &[
                "-c",
                "user.name=AutoPkg Test",
                "-c",
                "user.email=autopkg@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--no-gpg-sign",
                "-m",
                "fixture",
            ],
        )
        .unwrap();
        let options = Options {
            args: vec![source.to_string_lossy().into_owned()],
            ..Default::default()
        };
        repo_add(&mut store, &options).unwrap();
        let repos = repositories(&store.values).unwrap();
        assert_eq!(repos.len(), 1);
        let path = repos.keys().next().unwrap().clone();
        assert!(Path::new(&path).join("App.recipe").exists());
        assert_eq!(
            repo_update(
                &store,
                &Options {
                    args: vec!["all".into()],
                    ..Default::default()
                }
            )
            .unwrap(),
            0
        );
        repo_delete(
            &mut store,
            &Options {
                args: vec![path.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        #[cfg(not(windows))]
        assert!(!Path::new(&path).exists());
        assert!(source.exists());
        assert!(repositories(
            &Preferences::load(&temp.path().join("prefs.plist"))
                .unwrap()
                .values
        )
        .unwrap()
        .is_empty());
    }
}
