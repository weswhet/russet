//! `russet --install-helpers` and `russet --uninstall-helpers`: set up or
//! remove the launchd jobs that start the privileged helpers. Installing
//! Russet doesn't set them up; an administrator runs this when recipes need
//! PkgCreator or Installer.

use crate::Service;
use std::path::{Component, Path, PathBuf};

const SERVICES: [Service; 2] = [Service::Packaging, Service::Installation];
/// The executable path that the plist templates name.
const TEMPLATE_EXECUTABLE: &str = "<string>/opt/russet/russet</string>";

impl Service {
    /// The launchd job label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Packaging => "com.github.weswhet.russet.server",
            Self::Installation => "com.github.weswhet.russet.installd",
        }
    }

    /// The job's plist, the same one the archive's install.sh installs.
    fn template(self) -> &'static str {
        match self {
            Self::Packaging => include_str!("../../../distribution/launchd/russet-server.plist"),
            Self::Installation => {
                include_str!("../../../distribution/launchd/russet-installd.plist")
            }
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

/// The job's plist with `executable` in place of the template's path.
fn render(service: Service, executable: &Path) -> Result<String, String> {
    let path = executable
        .to_str()
        .ok_or_else(|| format!("{} isn't valid UTF-8", executable.display()))?;
    let escaped = path
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let template = service.template();
    if template.matches(TEMPLATE_EXECUTABLE).count() != 1 {
        return Err(format!("The {} job template is malformed", service.name()));
    }
    Ok(template.replace(TEMPLATE_EXECUTABLE, &format!("<string>{escaped}</string>")))
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
                .and_then(|mut file| file.write_all(text.as_bytes()).and(file.sync_all()))
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
    fn plists_name_the_executable_and_keep_the_rest_of_the_template() {
        for service in SERVICES {
            let text = render(service, Path::new("/opt/homebrew/opt/russet/bin/russet")).unwrap();
            assert!(text.contains("<string>/opt/homebrew/opt/russet/bin/russet</string>"));
            assert!(!text.contains("/opt/russet/russet"));
            assert!(text.contains(&format!("<string>{}</string>", service.label())));
            assert!(text.contains(&format!("<string>/var/run/{}</string>", service.name())));
            let unchanged = render(service, Path::new("/opt/russet/russet")).unwrap();
            assert_eq!(unchanged, service.template());
        }
        let odd = render(Service::Packaging, Path::new("/tmp/a&b<c>/russet")).unwrap();
        assert!(odd.contains("<string>/tmp/a&amp;b&lt;c&gt;/russet</string>"));
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
