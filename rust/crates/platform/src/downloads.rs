//! Shared native download tool discovery.
use plist::Dictionary;
use std::fs;
use std::path::PathBuf;

pub fn curl_binary(env: &Dictionary) -> Result<PathBuf, String> {
    fn executable(path: &std::path::Path) -> bool {
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
    if let Some(value) = env.get("CURL_PATH") {
        let path = PathBuf::from(value.as_string().ok_or("CURL_PATH must be a string")?);
        if executable(&path) {
            return Ok(path);
        }
        crate::text_eprintln!("WARNING: path given in the 'CURL_PATH' environment: '{}' either doesn't exist or is not executable! Continuing search for usable 'curl'.", path.display());
    }
    if let Some(value) = crate::preference("com.github.autopkg", "CURL_PATH")? {
        if let Some(path) = value.as_string() {
            let path = PathBuf::from(path);
            if executable(&path) {
                return Ok(path);
            }
        }
    }
    let binary = if cfg!(windows) { "curl.exe" } else { "curl" };
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let path = directory.join(binary);
        if executable(&path) {
            return Ok(path);
        }
    }
    if !cfg!(windows) && executable(std::path::Path::new("/usr/bin/curl")) {
        return Ok(PathBuf::from("/usr/bin/curl"));
    }
    Err("Unable to locate or execute any curl binary".into())
}
