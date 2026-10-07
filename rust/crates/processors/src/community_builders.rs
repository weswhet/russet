//! Native ports of the AutoPkg recipe builders (Apache-2.0).
//! Framework relocation follows gregneagle/relocatable-python at 8ee72fe3;
//! catalog generation follows Munki 7.2.0's native makecatalogs.
use plist::{Dictionary, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn output(message: impl AsRef<str>) {
    autopkg_platform::processor_output(1, message.as_ref());
}

const BUILDER_SHA: &str = "8ee72fe3a5dbef733365370ebf44f25022b895ef";
fn string<'a>(env: &'a Dictionary, key: &str) -> Result<&'a str, String> {
    env.get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| format!("Missing or invalid string input: {key}"))
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Boolean(v) => *v,
        Value::String(v) => !v.is_empty(),
        Value::Integer(v) => v.as_signed() != Some(0),
        Value::Real(v) => *v != 0.,
        Value::Array(v) => !v.is_empty(),
        Value::Dictionary(v) => !v.is_empty(),
        Value::Data(v) => !v.is_empty(),
        _ => true,
    }
}
pub(super) fn execute(
    name: &str,
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
) -> Result<(), String> {
    match name {
        "AutoPkgSourceFinder" => source_finder(env),
        "GenerateRelocatablePython" => generate_python(env),
        "MakeCatalogsProcessor" => make_catalogs(env, preferences),
        _ => Err(format!("Unknown community builder {name}")),
    }
}
fn source_finder(env: &mut Dictionary) -> Result<(), String> {
    let root = string(env, "input_path")?;
    // Python glob preserves directory enumeration order, and returns root/ on no match.
    let found = fs::read_dir(root)
        .ok()
        .and_then(|entries| {
            entries.filter_map(Result::ok).find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("autopkg-autopkg-")
            })
        })
        .map(|entry| entry.path());
    let path = found
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{}/", root.trim_end_matches('/')));
    output(format!("Found {path}"));
    env.insert("autopkg_path".into(), path.into());
    Ok(())
}
fn list_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            files.extend(list_files(&entry.path())?);
        } else {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}
// Munki FileRepo enumerates non-dot files before descending into directories,
// follows directory links, and treats absent resource directories as empty.
fn repo_files(root: &Path) -> Vec<PathBuf> {
    fn visit(root: &Path, ancestors: &mut Vec<PathBuf>, files: &mut Vec<PathBuf>) {
        let Ok(real) = fs::canonicalize(root) else {
            return;
        };
        if ancestors.contains(&real) {
            return;
        }
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        ancestors.push(real);
        let mut dirs = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.path().is_dir() {
                dirs.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
        for dir in dirs {
            visit(&dir, ancestors, files);
        }
        ancestors.pop();
    }
    let mut files = Vec::new();
    visit(root, &mut Vec::new(), &mut files);
    files
}
fn repo_path(value: &str) -> Result<PathBuf, String> {
    if value.starts_with("file:") {
        url::Url::parse(value)
            .map_err(|e| e.to_string())?
            .to_file_path()
            .map_err(|_| "Invalid FileRepo URL".into())
    } else if Path::new(value).is_absolute() {
        Ok(value.into())
    } else {
        Err("MakeCatalogsProcessor requires a local FileRepo path or file URL".into())
    }
}
fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn make_catalogs(env: &mut Dictionary, preferences: Option<&Dictionary>) -> Result<(), String> {
    autopkg_munki::validate_backend(env)?;
    let cache = preferences
        .and_then(|p| p.get("CACHE_DIR"))
        .and_then(Value::as_string)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Library/AutoPkg/Cache")
        });
    let changed = match fs::read(cache.join("autopkg_results.plist")) {
        Ok(bytes) => {
            let results = Value::from_reader(std::io::Cursor::new(bytes))
                .map_err(|e| format!("Invalid autopkg_results.plist: {e}"))?;
            results
                .as_array()
                .ok_or("autopkg_results.plist must contain an array")?
                .iter()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(Value::as_dictionary)
                .filter_map(|d| d.get("Output"))
                .filter_map(Value::as_dictionary)
                .any(|d| d.get("munki_repo_changed").is_some_and(truthy))
        }
        Err(_) => false,
    };
    if !changed && !env.get("force_rebuild").is_some_and(truthy) {
        output("No need to rebuild catalogs.");
        env.insert("makecatalogs_resultcode".into(), 0.into());
        env.insert("makecatalogs_stderr".into(), "".into());
        return Ok(());
    }
    let root = repo_path(string(env, "MUNKI_REPO")?)?;
    let result = rebuild_catalogs(&root);
    let (warnings, errors) = match result {
        Ok(v) => v,
        Err(e) => (Vec::new(), vec![e]),
    };
    let stderr = warnings
        .iter()
        .chain(&errors)
        .map(|s| format!("{s}\n"))
        .collect::<String>();
    env.insert(
        "makecatalogs_resultcode".into(),
        if errors.is_empty() {
            0.into()
        } else {
            1.into()
        },
    );
    env.insert("makecatalogs_stderr".into(), stderr.clone().into());
    if errors.is_empty() {
        output("Munki catalogs rebuilt!");
        Ok(())
    } else {
        Err(format!("makecatalogs failed: \n{stderr}"))
    }
}
fn verify_payload(
    info: &Dictionary,
    identifier: &str,
    packages: &[String],
    warnings: &mut Vec<String>,
) -> bool {
    if matches!(
        info.get("installer_type").and_then(Value::as_string),
        Some("nopkg" | "apple_update_metadata")
    ) || ["PackageCompleteURL", "PackageURL"].iter().any(|k| {
        info.get(k)
            .and_then(Value::as_string)
            .is_some_and(|s| !s.is_empty())
    }) {
        return true;
    }
    for (key, description) in [
        ("installer_item_location", "installer"),
        ("uninstaller_item_location", "uninstaller"),
    ] {
        if key.starts_with("uninstaller") && info.get(key).and_then(Value::as_string).is_none() {
            continue;
        }
        let location = info.get(key).and_then(Value::as_string).unwrap_or("");
        if location.is_empty() {
            warnings.push(format!("WARNING: empty or invalid {key} in {identifier}"));
            return false;
        }
        if !packages.iter().any(|p| p == location) {
            if let Some(found) = packages
                .iter()
                .find(|p| p.to_lowercase() == location.to_lowercase())
            {
                warnings.push(format!("WARNING: {identifier} refers to {description} item: {location}. The pathname of the item in the repo has different case: {found}. This may cause issues depending on the case-sensitivity of the underlying filesystem."));
            } else {
                warnings.push(format!(
                    "WARNING: {identifier} refers to missing {description} item: {location}"
                ));
                return false;
            }
        }
    }
    true
}
fn rebuild_catalogs(root: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    if !root.is_dir() {
        return Err(format!("Repo error: {} is not a directory", root.display()));
    }
    let info_paths = repo_files(&root.join("pkgsinfo"));
    let packages = repo_files(&root.join("pkgs"))
        .iter()
        .map(|p| {
            p.strip_prefix(root.join("pkgs"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    let (mut warnings, mut errors) = (Vec::new(), Vec::new());
    let mut catalogs: BTreeMap<String, Vec<Value>> = BTreeMap::from([("all".into(), Vec::new())]);
    for path in info_paths {
        let identifier = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut info = match Value::from_file(&path) {
            Ok(value) => value.into_dictionary().unwrap_or_default(),
            Err(e) => {
                errors.push(format!("Unexpected error reading {identifier}: {e}"));
                continue;
            }
        };
        if !info.contains_key("name") {
            warnings.push(format!("WARNING: {identifier} is missing name key"));
            continue;
        }
        info.retain(|key, _| key != "notes" && !key.starts_with('_'));
        if !verify_payload(&info, &identifier, &packages, &mut warnings) {
            continue;
        }
        let value = Value::Dictionary(info.clone());
        catalogs.get_mut("all").unwrap().push(value.clone());
        let names = info
            .get("catalogs")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(Value::as_string).collect::<Option<Vec<_>>>());
        match names {
            Some(names) if names.is_empty() => warnings.push(format!(
                "WARNING: {identifier} has an empty catalogs array!"
            )),
            Some(names) => {
                for name in names {
                    if !safe_relative(name) {
                        errors.push(format!("Invalid catalog name: {name}"));
                        continue;
                    }
                    catalogs.entry(name.into()).or_default().push(value.clone());
                }
            }
            None => warnings.push(format!("WARNING: {identifier} has no catalogs array!")),
        }
    }
    let duplicates: Vec<_> = catalogs
        .keys()
        .filter(|name| {
            catalogs
                .keys()
                .any(|other| other != *name && other.to_lowercase() == name.to_lowercase())
        })
        .collect();
    if !duplicates.is_empty() {
        warnings.push(format!("WARNING: There are catalogs with names that differ only by case. This may cause issues depending on the case-sensitivity of the underlying filesystem: {duplicates:?}"));
    }
    for old in repo_files(&root.join("catalogs")) {
        let name = old
            .strip_prefix(root.join("catalogs"))
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !catalogs.contains_key(&name) {
            if let Err(e) = fs::remove_file(&old) {
                errors.push(format!("Could not delete catalog {name}: {e}"));
            }
        }
    }
    for (name, items) in catalogs {
        if items.is_empty() {
            continue;
        } // Munki 7 retains an existing empty catalog.
        let path = root.join("catalogs").join(&name);
        let result = fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| e.to_string())
            .and_then(|_| {
                Value::Array(items)
                    .to_file_xml(&path)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            errors.push(format!("Failed to create catalog {name}: {e}"));
        }
    }
    let mut hashes = Dictionary::new();
    {
        let icons = repo_files(&root.join("icons"));
        for path in icons {
            let name = path
                .strip_prefix(root.join("icons"))
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if name == "_icon_hashes.plist" {
                continue;
            }
            match fs::read(&path) {
                Ok(bytes) => {
                    hashes.insert(name, format!("{:x}", Sha256::digest(bytes)).into());
                }
                Err(e) => errors.push(format!("Error reading icons/{name}: {e}")),
            }
        }
    }
    if !hashes.is_empty() {
        if let Err(e) = Value::Dictionary(hashes).to_file_xml(root.join("icons/_icon_hashes.plist"))
        {
            errors.push(format!("Failed to create icons/_icon_hashes.plist: {e}"));
        }
    }
    Ok((warnings, errors))
}

fn command(cmd: &mut Command, seconds: Option<u64>) -> Result<String, String> {
    let label = format!("{cmd:?}");
    autopkg_platform::processor_output(4, format!("Command: {label}"));
    // File-backed capture avoids pipe deadlocks during long pip and git operations.
    let stdout = tempfile::tempfile().map_err(|e| e.to_string())?;
    let stderr = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = cmd
        .stdout(Stdio::from(stdout.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(stderr.try_clone().map_err(|e| e.to_string())?))
        .spawn()
        .map_err(|e| format!("Could not run {label}: {e}"))?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if seconds.is_some_and(|limit| start.elapsed() >= Duration::from_secs(limit)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Command timed out: {label}"));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    use std::io::{Read, Seek};
    let read = |mut file: fs::File| -> Result<String, String> {
        file.rewind().map_err(|e| e.to_string())?;
        let mut value = String::new();
        file.read_to_string(&mut value).map_err(|e| e.to_string())?;
        Ok(value)
    };
    let out = read(stdout)?;
    let err = read(stderr)?;
    if status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{label} failed with exit code {}: {err}{out}",
            status.code().unwrap_or(-1)
        ))
    }
}
fn generate_python(env: &mut Dictionary) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("GenerateRelocatablePython requires macOS".into());
    }
    let version = string(env, "python_version")?;
    let os = string(env, "os_version")?;
    let requirements = string(env, "requirements_path")?;
    let sha = env
        .get("relocatable_python_sha")
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .unwrap_or(BUILDER_SHA);
    if sha != BUILDER_SHA {
        return Err(format!(
            "Unsupported relocatable_python_sha {sha}; native builder supports {BUILDER_SHA}"
        ));
    }
    let short = version.split('.').take(2).collect::<Vec<_>>().join(".");
    if version.split('.').count() < 2
        || !version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.')
        || !os.chars().all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err("Invalid Python or OS version".into());
    }
    let cache = PathBuf::from(string(env, "RECIPE_CACHE_DIR")?);
    fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let cache = fs::canonicalize(cache).map_err(|e| e.to_string())?;
    let checkout = cache.join("relocatable-python");
    if checkout.exists() {
        fs::remove_dir_all(&checkout).map_err(|e| e.to_string())?;
    }
    output(format!(
        "Cloning Relocatable Python into {}",
        checkout.display()
    ));
    command(
        Command::new("git")
            .args([
                "clone",
                "https://github.com/gregneagle/relocatable-python.git",
            ])
            .arg(&checkout),
        Some(3600),
    )?;
    if env.get("relocatable_python_sha").is_some_and(truthy) {
        output(format!("Checking out relocatable-python at {sha}"));
    }
    command(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["checkout", sha]),
        Some(60),
    )?;
    let resolved = command(
        Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "HEAD"]),
        Some(60),
    )?;
    if resolved.trim() != BUILDER_SHA {
        return Err("Relocatable Python checkout does not match the native implementation".into());
    }
    let dest = cache.join("Python.framework");
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
    }
    output("Building relocatable python framework...");
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let package = temp.path().join("python.pkg");
    let platform = if os.starts_with("10") {
        "macosx"
    } else {
        "macos"
    };
    let url =
        format!("https://www.python.org/ftp/python/{version}/python-{version}-{platform}{os}.pkg");
    command(
        Command::new("/usr/bin/curl")
            .args(["--fail", "-o"])
            .arg(&package)
            .arg(url),
        None,
    )?;
    let expanded = temp.path().join("expanded");
    command(
        Command::new("/usr/sbin/pkgutil")
            .arg("--expand")
            .arg(&package)
            .arg(&expanded),
        None,
    )?;
    command(
        Command::new("/usr/bin/ditto")
            .arg("-xz")
            .arg(expanded.join("Python_Framework.pkg/Payload"))
            .arg(&dest),
        None,
    )?;
    relocate_framework(&dest)?;
    let python = dest.join(format!("Versions/{short}/bin/python{short}"));
    command(Command::new(&python).args(["-s", "-m", "ensurepip"]), None)?;
    command(
        Command::new(&python).args(["-s", "-m", "pip", "install", "wheel"]),
        None,
    )?;
    if env.get("upgrade_pip").is_some_and(truthy) {
        command(
            Command::new(&python).args(["-s", "-m", "pip", "install", "--upgrade", "pip"]),
            None,
        )?;
    }
    if !requirements.is_empty() {
        command(
            Command::new(&python)
                .args(["-s", "-m", "pip", "install", "-r", requirements])
                .env(
                    "CPPFLAGS",
                    format!(
                        "-I{}",
                        dest.join(format!("Versions/{short}/include/python{short}"))
                            .display()
                    ),
                ),
            None,
        )?;
    }
    fix_scripts(&dest, &short)?;
    output(format!("Framework built at {}", dest.display()));
    install_sitecustomize(&dest, &short)?;
    output("Smoke-testing HTTPS from built framework...");
    command(Command::new(&python).args(["-c", "import urllib.request; urllib.request.urlopen('https://example.com', timeout=15).close()"]), Some(30)).map_err(|e| format!("HTTPS smoke test failed: {e}"))?;
    output("HTTPS smoke test passed.");
    env.insert(
        "python_path".into(),
        dest.to_string_lossy().into_owned().into(),
    );
    Ok(())
}
#[derive(Clone)]
struct MachFile {
    path: PathBuf,
    install: String,
    dependencies: Vec<String>,
    executable: bool,
    dylib: bool,
}
fn install_name(path: &Path) -> Result<String, String> {
    Ok(
        command(Command::new("/usr/bin/otool").arg("-D").arg(path), None)?
            .lines()
            .nth(1)
            .unwrap_or("")
            .into(),
    )
}
fn framework_root(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| p.extension().is_some_and(|s| s == "framework"))
}
fn relative_path(target: &Path, base: &Path) -> PathBuf {
    let target: Vec<_> = target.components().collect();
    let base: Vec<_> = base.components().collect();
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut result = PathBuf::new();
    for _ in common..base.len() {
        result.push("..");
    }
    for item in &target[common..] {
        result.push(item.as_os_str());
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}
fn relocate_framework(root: &Path) -> Result<(), String> {
    command(
        Command::new("/bin/chmod")
            .args(["-R", "u+rw,g+r,g-w,o+r,o-w"])
            .arg(root),
        None,
    )?;
    let mut prefix = String::new();
    for entry in fs::read_dir(root.join("Versions")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir()
            && entry.path().join("Python").exists()
        {
            let name = install_name(&entry.path().join("Python"))?;
            if !name.starts_with('@') {
                prefix = framework_root(Path::new(&name))
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                break;
            }
        }
    }
    let mut files = Vec::new();
    for path in list_files(root)? {
        if fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let kind = if ext == "so" || ext == "dylib" {
            String::new()
        } else {
            command(Command::new("/usr/bin/file").arg("-b").arg(&path), None)?
        };
        let executable = kind.contains("Mach-O 64-bit executable");
        let dylib =
            ext == "dylib" || kind.contains("Mach-O 64-bit dynamically linked shared library");
        if ext != "so" && !executable && !dylib {
            continue;
        }
        let install = install_name(&path)?;
        let output = command(Command::new("/usr/bin/otool").arg("-L").arg(&path), None)?;
        let dependencies: Vec<_> = output
            .lines()
            .skip(if install.is_empty() { 1 } else { 2 })
            .map(|line| {
                line.trim_start()
                    .split(" (compatibility")
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        if install.starts_with(&prefix) || dependencies.iter().any(|s| s.starts_with(&prefix)) {
            files.push(MachFile {
                path,
                install,
                dependencies,
                executable,
                dylib,
            });
        }
    }
    let mut changed = std::collections::BTreeSet::new();
    for file in files.iter().filter(|file| file.dylib) {
        let new = if !file.install.is_empty() && !file.install.starts_with('@') {
            format!(
                "@rpath/{}",
                file.path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .display()
            )
        } else {
            file.install.clone()
        };
        if new != file.install {
            command(
                Command::new("/usr/bin/install_name_tool")
                    .args(["-id", &new])
                    .arg(&file.path),
                None,
            )?;
        }
        changed.insert(file.path.clone());
        if new != file.install {
            for item in &files {
                if item.dependencies.contains(&file.install) {
                    command(
                        Command::new("/usr/bin/install_name_tool")
                            .args(["-change", &file.install, &new])
                            .arg(&item.path),
                        None,
                    )?;
                    changed.insert(item.path.clone());
                }
            }
        }
    }
    for file in files.iter().filter(|file| file.executable) {
        let rpath = format!(
            "@executable_path/{}/",
            relative_path(root, file.path.parent().unwrap()).display()
        );
        let load = command(
            Command::new("/usr/bin/otool").arg("-l").arg(&file.path),
            None,
        )?;
        if !load.lines().any(|line| {
            line.trim()
                .strip_prefix("path ")
                .and_then(|s| s.split(" (offset ").next())
                == Some(&rpath)
        }) {
            command(
                Command::new("/usr/bin/install_name_tool")
                    .args(["-add_rpath", &rpath])
                    .arg(&file.path),
                None,
            )?;
        }
        changed.insert(file.path.clone());
    }
    for path in changed {
        command(
            Command::new("/usr/bin/codesign")
                .args([
                    "-s",
                    "-",
                    "--deep",
                    "--force",
                    "--preserve-metadata=identifier,entitlements,flags,runtime",
                ])
                .arg(path),
            None,
        )?;
    }
    Ok(())
}
fn fix_scripts(root: &Path, short: &str) -> Result<(), String> {
    let current = root.join("Versions/Current");
    if !current.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(short, current).map_err(|e| e.to_string())?;
    }
    let bin = root.join(format!("Versions/{short}/bin"));
    for path in list_files(&bin)? {
        if fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            continue;
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        let end = bytes
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(bytes.len());
        let first = String::from_utf8_lossy(&bytes[..end]);
        if let Some(interpreter) = first.trim().strip_prefix("#!") {
            if [
                root.to_string_lossy().as_ref(),
                "/Library/Frameworks/Python.framework",
                "/Library/Developer/CommandLineTools/usr/bin/python3",
                "/Applications/Xcode.app/Contents/Developer/usr/bin/python3",
            ]
            .iter()
            .any(|prefix| interpreter.starts_with(prefix))
            {
                let normalized = interpreter.replacen(
                    "/Library/Frameworks/Python.framework",
                    &root.to_string_lossy(),
                    1,
                );
                let canonical =
                    fs::canonicalize(&normalized).unwrap_or_else(|_| PathBuf::from(normalized));
                let relative = relative_path(
                    &canonical,
                    &fs::canonicalize(&bin).map_err(|e| e.to_string())?,
                )
                .to_string_lossy()
                .into_owned();
                let header = format!("#!/bin/sh\n'''exec' \"$(dirname \"$0\")/{relative}\" \"$0\" \"$@\"\n' '''\n# the above calls the {relative} interpreter relative to the directory of this script\n");
                let mut rewritten = header.into_bytes();
                rewritten.extend_from_slice(&bytes[(end + 1).min(bytes.len())..]);
                fs::write(path, rewritten).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
const SITECUSTOMIZE: &str = "import os\n\n\ndef _ssl_cert_file_is_valid():\n    path = os.environ.get('SSL_CERT_FILE')\n    return bool(path) and os.path.isfile(path)\n\n\nif not _ssl_cert_file_is_valid():\n    try:\n        import certifi\n    except ImportError:\n        pass\n    else:\n        os.environ['SSL_CERT_FILE'] = certifi.where()\n";
fn install_sitecustomize(root: &Path, short: &str) -> Result<(), String> {
    let site = root.join(format!("Versions/{short}/lib/python{short}/site-packages"));
    fs::create_dir_all(&site).map_err(|e| e.to_string())?;
    let site_file = site.join("sitecustomize.py");
    if !site_file.exists() {
        fs::write(&site_file, SITECUSTOMIZE).map_err(|e| e.to_string())?;
        output(format!(
            "Installed sitecustomize.py at {}",
            site_file.display()
        ));
    } else {
        output(format!(
            "sitecustomize.py already exists at {}; skipping",
            site_file.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_finder_matches_and_preserves_no_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([(
            "input_path",
            Value::String(dir.path().display().to_string()),
        )]);
        source_finder(&mut env).unwrap();
        assert_eq!(
            env["autopkg_path"].as_string().unwrap(),
            format!("{}/", dir.path().display())
        );
        fs::create_dir(dir.path().join("autopkg-autopkg-abc")).unwrap();
        source_finder(&mut env).unwrap();
        assert!(env["autopkg_path"]
            .as_string()
            .unwrap()
            .ends_with("autopkg-autopkg-abc"));
    }
    #[test]
    fn catalogs_strip_private_data_hash_icons_and_remove_stale_catalogs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for folder in ["pkgs", "pkgsinfo", "catalogs", "icons"] {
            fs::create_dir(root.join(folder)).unwrap();
        }
        let info = Dictionary::from_iter([
            ("name", "Example".into()),
            ("version", "1".into()),
            ("installer_type", "nopkg".into()),
            ("notes", "private".into()),
            ("_metadata", "private".into()),
            ("catalogs", Value::Array(vec!["testing".into()])),
        ]);
        Value::Dictionary(info.clone())
            .to_file_xml(root.join("pkgsinfo/item"))
            .unwrap();
        fs::write(root.join("icons/example.png"), b"icon").unwrap();
        fs::write(root.join("catalogs/obsolete"), b"stale").unwrap();
        let (warnings, errors) = rebuild_catalogs(root).unwrap();
        assert!(warnings.is_empty());
        assert!(errors.is_empty());
        assert!(!root.join("catalogs/obsolete").exists());
        let all = Value::from_file(root.join("catalogs/all")).unwrap();
        let result = all.as_array().unwrap()[0].as_dictionary().unwrap();
        assert!(!result.contains_key("notes"));
        assert!(!result.contains_key("_metadata"));
        assert_eq!(
            Value::from_file(root.join("pkgsinfo/item")).unwrap(),
            Value::Dictionary(info)
        );
        let hashes = Value::from_file(root.join("icons/_icon_hashes.plist")).unwrap();
        assert_eq!(
            hashes.as_dictionary().unwrap()["example.png"]
                .as_string()
                .unwrap(),
            format!("{:x}", Sha256::digest(b"icon"))
        );
        assert_eq!(
            Value::from_file(root.join("catalogs/testing")).unwrap(),
            all
        );
    }
    #[test]
    fn cached_results_trigger_rebuild_and_false_flag_skips() {
        let dir = tempfile::tempdir().unwrap();
        let prefs =
            Dictionary::from_iter([("CACHE_DIR", Value::String(dir.path().display().to_string()))]);
        let mut env = Dictionary::from_iter([(
            "MUNKI_REPO",
            Value::String(
                dir.path()
                    .join("missing-test-repository")
                    .display()
                    .to_string(),
            ),
        )]);
        make_catalogs(&mut env, Some(&prefs)).unwrap();
        assert_eq!(env["makecatalogs_resultcode"].as_signed_integer(), Some(0));
        let output = Value::Dictionary(Dictionary::from_iter([(
            "munki_repo_changed",
            Value::Boolean(true),
        )]));
        let item = Value::Dictionary(Dictionary::from_iter([("Output", output)]));
        Value::Array(vec![Value::Array(vec![item])])
            .to_file_xml(dir.path().join("autopkg_results.plist"))
            .unwrap();
        assert!(make_catalogs(&mut env, Some(&prefs)).is_err());
        assert_eq!(env["makecatalogs_resultcode"].as_signed_integer(), Some(1));
    }
    #[test]
    fn verifies_missing_and_case_mismatched_payloads() {
        let info =
            Dictionary::from_iter([("installer_item_location", Value::String("item.pkg".into()))]);
        let mut warnings = Vec::new();
        assert!(!verify_payload(
            &info,
            "pkgsinfo/example",
            &[],
            &mut warnings
        ));
        assert!(warnings[0].contains("missing installer"));
        warnings.clear();
        assert!(verify_payload(
            &info,
            "pkgsinfo/example",
            &["Item.pkg".into()],
            &mut warnings
        ));
        assert!(warnings[0].contains("different case"));
    }
    #[test]
    fn sitecustomize_is_preserved_and_relative_paths_work() {
        let dir = tempfile::tempdir().unwrap();
        install_sitecustomize(dir.path(), "3.13").unwrap();
        let path = dir
            .path()
            .join("Versions/3.13/lib/python3.13/site-packages/sitecustomize.py");
        assert_eq!(fs::read_to_string(&path).unwrap(), SITECUSTOMIZE);
        fs::write(&path, "existing").unwrap();
        install_sitecustomize(dir.path(), "3.13").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "existing");
        assert_eq!(
            relative_path(
                Path::new("/a/Python.framework"),
                Path::new("/a/Python.framework/Versions/3.13/bin")
            ),
            Path::new("../../..")
        );
    }
    #[cfg(unix)]
    #[test]
    fn relocates_script_shebang_and_retains_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Python.framework");
        let bin = root.join("Versions/3.13/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("python3.13"), "binary placeholder").unwrap();
        let script = bin.join("pip3");
        fs::write(
            &script,
            "#!/Library/Frameworks/Python.framework/Versions/3.13/bin/python3.13\nprint('hello')\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        fix_scripts(&root, "3.13").unwrap();
        let text = fs::read_to_string(&script).unwrap();
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("/python3.13"));
        assert!(text.ends_with("print('hello')\n"));
        assert_eq!(
            fs::metadata(script).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_link(root.join("Versions/Current")).unwrap(),
            Path::new("3.13")
        );
    }
    #[test]
    fn catalog_hidden_files_and_missing_resource_directories_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pkgsinfo")).unwrap();
        fs::write(dir.path().join("pkgsinfo/.DS_Store"), "not a plist").unwrap();
        let (warnings, errors) = rebuild_catalogs(dir.path()).unwrap();
        assert!(warnings.is_empty());
        assert!(errors.is_empty());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn unknown_builder_revision_is_rejected_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = Dictionary::from_iter([
            ("python_version", "3.13.1".into()),
            ("os_version", "11".into()),
            ("requirements_path", "requirements.txt".into()),
            ("relocatable_python_sha", "unsupported".into()),
            (
                "RECIPE_CACHE_DIR",
                Value::String(dir.path().display().to_string()),
            ),
        ]);
        assert!(generate_python(&mut env)
            .unwrap_err()
            .contains("Unsupported relocatable_python_sha"));
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(!env.contains_key("python_path"));
    }
}
