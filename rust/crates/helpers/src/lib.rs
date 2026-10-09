//! Native helper protocols. `russet --server` and `russet --installd` run the services.
// The helper services run only on macOS, but their request checks also back
// the Linux package builder in `native`.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod common;
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod copier;
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod installer;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod launchd;
#[cfg(unix)]
mod native;
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod packager;
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod runtime;
pub use launchd::{install as install_launchd_jobs, uninstall as uninstall_launchd_jobs};

#[derive(Clone, Copy, Debug)]
pub enum Service {
    Packaging,
    Installation,
}
impl Service {
    /// The launchd socket name, the log file name in `/private/var/log`, and
    /// the socket file name in `/var/run`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Packaging => "russet-server",
            Self::Installation => "russet-installd",
        }
    }
    pub fn socket_path(self) -> std::path::PathBuf {
        std::path::Path::new("/var/run").join(self.name())
    }
}

pub fn run(service: Service) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        runtime::run(service)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = service;
        Err("Russet helper services require macOS".into())
    }
}

#[cfg(any(target_os = "macos", all(test, unix)))]
fn request_at(
    path: &std::path::Path,
    request: &plist::Dictionary,
    installation: bool,
) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    struct DisconnectOutput;
    impl Drop for DisconnectOutput {
        fn drop(&mut self) {
            autopkg_platform::processor_output(1, "Disconnecting");
        }
    }
    let _disconnect_output = DisconnectOutput;
    autopkg_platform::processor_output(1, "Connecting");
    let mut stream = UnixStream::connect(path)
        .map_err(|e| format!("Couldn't connect to {}: {e}", path.display()))?;
    autopkg_platform::processor_output(
        1,
        if installation {
            "Sending installation request"
        } else {
            "Sending packaging request"
        },
    );
    let mut bytes = Vec::new();
    plist::Value::Dictionary(request.clone())
        .to_writer_xml(&mut bytes)
        .map_err(|e| e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    if !installation {
        let mut reply = String::new();
        reader
            .read_to_string(&mut reply)
            .map_err(|e| e.to_string())?;
        if let Some(path) = reply.strip_prefix("OK:") {
            return Ok(path.trim_end().into());
        }
        if reply.trim().is_empty() {
            return Err("No reply from server (crash?), check system logs".into());
        }
        return Err(reply
            .trim_end()
            .lines()
            .map(|s| s.replace("ERROR:", ""))
            .collect::<Vec<_>>()
            .join(", "));
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("No reply from russet-installd (crash?), check system logs".into());
        }
        if let Some(result) = line.strip_prefix("OK:") {
            return Ok(result.trim_end().into());
        }
        if line.starts_with("ERROR:") {
            return Err(line.trim_end().replace("ERROR:", ""));
        }
        autopkg_platform::processor_output(1, line.trim_end());
    }
}
/// Builds a package: through the `russet-server` helper on macOS, or with
/// Russet's in-process builder on Linux and when `RUSSET_NATIVE` names
/// `pkgbuild`.
pub fn packaging_request(request: &plist::Dictionary) -> Result<String, String> {
    #[cfg(unix)]
    {
        use autopkg_platform::backend::{select, Backend, Tool};
        match select(Tool::Pkgbuild) {
            #[cfg(target_os = "macos")]
            Backend::Apple => return request_at(&Service::Packaging.socket_path(), request, false),
            Backend::Native => return native::package(request),
            _ => {}
        }
    }
    let _ = request;
    Err("Package creation is only supported on macOS and Linux".into())
}
pub fn installation_request(request: &plist::Dictionary) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        request_at(&Service::Installation.socket_path(), request, true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err("Installation through russet-installd requires macOS".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
        thread,
    };
    fn mock_reply(response: &'static [u8], installation: bool) -> Result<String, String> {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("socket");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0u8; 8192];
            let count = socket.read(&mut bytes).unwrap();
            let value = plist::Value::from_reader(std::io::Cursor::new(&bytes[..count])).unwrap();
            assert_eq!(
                value.as_dictionary().unwrap()["test"].as_boolean(),
                Some(true)
            );
            socket.write_all(response).unwrap();
        });
        let result = request_at(
            &path,
            &plist::Dictionary::from_iter([("test", true)]),
            installation,
        );
        worker.join().unwrap();
        result
    }
    #[test]
    fn packaging_client_reads_complete_response_and_errors() {
        assert_eq!(
            mock_reply(b"OK:/tmp/example.pkg\n", false).unwrap(),
            "/tmp/example.pkg"
        );
        assert_eq!(
            mock_reply(b"ERROR:first\nERROR:second\n", false).unwrap_err(),
            "first, second"
        );
        assert_eq!(
            mock_reply(b"plain worker failure\n", false).unwrap_err(),
            "plain worker failure"
        );
    }
    #[test]
    fn installer_client_consumes_progress_and_preserves_terminal_result() {
        assert_eq!(
            mock_reply(b"STATUS:installing\nOK:DONE\n", true).unwrap(),
            "DONE"
        );
        assert_eq!(
            mock_reply(b"STATUS:installing\nERROR:ERROR:failure\n", true).unwrap_err(),
            "failure"
        );
    }
}
