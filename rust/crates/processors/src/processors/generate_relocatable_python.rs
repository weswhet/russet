//! `GenerateRelocatablePython`: build a relocatable Python framework. A
//! native port of the autopkg/recipes processor (Apache-2.0); framework
//! relocation follows gregneagle/relocatable-python at 8ee72fe3.
//!
//! Inputs and outputs: run `russet processor-info GenerateRelocatablePython`, or see
//! `GenerateRelocatablePython` in `compatibility/community-processors.json`.
use crate::community_builders::{output, string, truthy};
use plist::{Dictionary, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BUILDER_SHA: &str = "8ee72fe3a5dbef733365370ebf44f25022b895ef";
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
pub(crate) fn execute(env: &mut Dictionary) -> Result<(), String> {
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
pub(crate) fn relative_path(target: &Path, base: &Path) -> PathBuf {
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
pub(crate) fn fix_scripts(root: &Path, short: &str) -> Result<(), String> {
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
pub(crate) const SITECUSTOMIZE: &str = "import os\n\n\ndef _ssl_cert_file_is_valid():\n    path = os.environ.get('SSL_CERT_FILE')\n    return bool(path) and os.path.isfile(path)\n\n\nif not _ssl_cert_file_is_valid():\n    try:\n        import certifi\n    except ImportError:\n        pass\n    else:\n        os.environ['SSL_CERT_FILE'] = certifi.where()\n";
pub(crate) fn install_sitecustomize(root: &Path, short: &str) -> Result<(), String> {
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
