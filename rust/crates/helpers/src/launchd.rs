//! `russet --install-helpers` and `russet --uninstall-helpers`: set up or
//! remove the launchd jobs that start the privileged helpers. Installing
//! Russet doesn't set them up; an administrator runs this when recipes need
//! PkgCreator or Installer.

use crate::Service;
use plist::Value;
use std::path::{Component, Path, PathBuf};

const SERVICES: [Service; 2] = [Service::Packaging, Service::Installation];

impl Service {
    /// The launchd job label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Packaging => "com.github.weswhet.russet.server",
            Self::Installation => "com.github.weswhet.russet.installd",
        }
    }

    /// The job definition in `rust/distribution/launchd`, which
    /// `cargo xtask package` also turns into the archive's plists. Its first
    /// program argument is the executable, which installation replaces.
    fn definition(self) -> &'static str {
        match self {
            Self::Packaging => {
                include_str!("../../../distribution/launchd/com.github.weswhet.russet.server.json")
            }
            Self::Installation => include_str!(
                "../../../distribution/launchd/com.github.weswhet.russet.installd.json"
            ),
        }
    }

    fn plist_path(self) -> PathBuf {
        Path::new("/Library/LaunchDaemons").join(format!("{}.plist", self.label()))
    }
}

/// The path launchd runs. A Homebrew installation lives in a versioned
/// `Cellar/russet/VERSION` folder that an upgrade removes, so its jobs name
/// Homebrew's stable `opt/russet` link instead.
fn job_executable(executable: &Path) -> PathBuf {
    let parts: Vec<Component> = executable.components().collect();
    let cellar = parts.windows(5).position(|window| {
        window[0].as_os_str() == "Cellar"
            && window[1].as_os_str() == "russet"
            && window[3].as_os_str() == "bin"
            && window[4].as_os_str() == "russet"
    });
    match cellar {
        Some(index) if index + 5 == parts.len() => parts[..index]
            .iter()
            .collect::<PathBuf>()
            .join("opt/russet/bin/russet"),
        _ => executable.to_path_buf(),
    }
}

/// The job's definition with `executable` as its program.
fn job(service: Service, executable: &Path) -> Result<Value, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| format!("{} isn't valid UTF-8", executable.display()))?;
    let malformed = |detail: &str| format!("The {} job definition {detail}", service.label());
    let mut job: Value = serde_json::from_str(service.definition())
        .map_err(|error| malformed(&format!("isn't valid JSON: {error}")))?;
    let dictionary = job
        .as_dictionary_mut()
        .ok_or_else(|| malformed("isn't an object"))?;
    if dictionary.get("Label").and_then(Value::as_string) != Some(service.label()) {
        return Err(malformed("has the wrong Label"));
    }
    let program = dictionary
        .get_mut("ProgramArguments")
        .and_then(Value::as_array_mut)
        .and_then(|arguments| arguments.first_mut())
        .ok_or_else(|| malformed("has no ProgramArguments"))?;
    *program = path.into();
    Ok(job)
}

/// The job's plist with `executable` as its program.
fn render(service: Service, executable: &Path) -> Result<Vec<u8>, String> {
    let mut xml = Vec::new();
    job(service, executable)?
        .to_writer_xml(&mut xml)
        .map_err(|error| format!("Unable to write the {} job: {error}", service.label()))?;
    Ok(xml)
}

#[cfg(target_os = "macos")]
fn require_root(flag: &str) -> Result<(), String> {
    // SAFETY: geteuid has no preconditions and can't fail.
    if unsafe { libc::geteuid() } != 0 {
        return Err(format!("russet {flag} must run as root; use sudo"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn launchctl(arguments: &[&str]) -> Result<bool, String> {
    std::process::Command::new("/bin/launchctl")
        .args(arguments)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("Unable to run launchctl: {error}"))
}

#[cfg(target_os = "macos")]
fn unload(service: Service) -> Result<(), String> {
    // Fails when the job isn't loaded, which is fine.
    launchctl(&["bootout", &format!("system/{}", service.label())]).map(drop)
}

/// Write each job's plist for this executable and load it, replacing any
/// earlier setup.
pub fn install() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};
        require_root("--install-helpers")?;
        let executable = std::env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|error| format!("Unable to find the russet executable: {error}"))?;
        let executable = job_executable(&executable);
        for service in SERVICES {
            let path = service.plist_path();
            let text = render(service, &executable)?;
            unload(service)?;
            let staged = path.with_extension("plist.new");
            let _ = fs::remove_file(&staged);
            let written = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&staged)
                .and_then(|mut file| file.write_all(&text).and(file.sync_all()))
                .and_then(|()| fs::rename(&staged, &path));
            if let Err(error) = written {
                let _ = fs::remove_file(&staged);
                return Err(format!("Unable to write {}: {error}", path.display()));
            }
            let path_text = path.to_string_lossy();
            if !launchctl(&["bootstrap", "system", &path_text])? {
                return Err(format!("launchctl couldn't load {path_text}"));
            }
            autopkg_platform::text_println!(
                "Loaded {} ({}) for {}",
                service.label(),
                service.socket_path().display(),
                executable.display()
            );
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Russet helper services require macOS".into())
}

/// Unload each job and remove its plist.
pub fn uninstall() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        require_root("--uninstall-helpers")?;
        for service in SERVICES {
            unload(service)?;
            let path = service.plist_path();
            match std::fs::remove_file(&path) {
                Ok(()) => autopkg_platform::text_println!("Removed {}", service.label()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Unable to remove {}: {error}", path.display())),
            }
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Russet helper services require macOS".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plists_run_the_executable_on_the_service_socket() {
        for (service, flag) in SERVICES.into_iter().zip(["--server", "--installd"]) {
            let executable = "/tmp/a&b<c>/bin/russet";
            let xml = render(service, Path::new(executable)).unwrap();
            let job = Value::from_reader_xml(xml.as_slice()).unwrap();
            let job = job.as_dictionary().unwrap();
            let get = |key: &str| job.get(key).unwrap();
            assert_eq!(get("Label").as_string(), Some(service.label()));
            assert_eq!(get("KeepAlive").as_boolean(), Some(false));
            let arguments: Vec<_> = get("ProgramArguments")
                .as_array()
                .unwrap()
                .iter()
                .map(|argument| argument.as_string().unwrap())
                .collect();
            assert_eq!(arguments, [executable, flag]);
            let socket = get("Sockets")
                .as_dictionary()
                .unwrap()
                .get(service.name())
                .unwrap()
                .as_dictionary()
                .unwrap();
            // launchd's socket path, written with `/` on every platform.
            let path = format!("/var/run/{}", service.name());
            assert_eq!(
                socket.get("SockPathName").unwrap().as_string(),
                Some(path.as_str())
            );
            assert_eq!(
                socket.get("SockPathMode").unwrap().as_signed_integer(),
                Some(0o666)
            );
        }
    }

    #[test]
    fn homebrew_jobs_name_the_stable_opt_link() {
        assert_eq!(
            job_executable(Path::new("/opt/homebrew/Cellar/russet/0.1.0/bin/russet")),
            Path::new("/opt/homebrew/opt/russet/bin/russet")
        );
        assert_eq!(
            job_executable(Path::new("/usr/local/Cellar/russet/0.1.0_1/bin/russet")),
            Path::new("/usr/local/opt/russet/bin/russet")
        );
        for path in [
            "/opt/russet/russet",
            "/opt/homebrew/Cellar/other/1.0/bin/russet",
            "/x/Cellar/russet/1.0/bin/russet/extra",
        ] {
            assert_eq!(job_executable(Path::new(path)), Path::new(path));
        }
    }
}
