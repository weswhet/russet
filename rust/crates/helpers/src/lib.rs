//! Native helper protocols. Development binaries are never installed implicitly.
#[cfg(any(target_os = "macos", all(test, unix)))]
mod common;
#[cfg(any(target_os = "macos", all(test, unix)))]
mod copier;
#[cfg(any(target_os = "macos", all(test, unix)))]
mod installer;
#[cfg(any(target_os = "macos", all(test, unix)))]
mod packager;
#[cfg(any(target_os = "macos", all(test, unix)))]
mod runtime;

#[derive(Clone, Copy, Debug)]
pub enum Service {
    Packaging,
    Installation,
}
impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Self::Packaging => "autopkgserver",
            Self::Installation => "autopkginstalld",
        }
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
        Err("AutoPkg privileged helpers require macOS".into())
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
            return Err("No reply from autopkginstalld (crash?), check system logs".into());
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
pub fn packaging_request(request: &plist::Dictionary) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        request_at(
            std::path::Path::new("/var/run/autopkgserver"),
            request,
            false,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err("Package creation through autopkgserver requires macOS".into())
    }
}
pub fn installation_request(request: &plist::Dictionary) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        request_at(
            std::path::Path::new("/var/run/autopkginstalld"),
            request,
            true,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        Err("Installation through autopkginstalld requires macOS".into())
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
