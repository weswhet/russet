use crate::{copier, installer, packager, Service};
use plist::{Dictionary, Value};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::Path,
};

fn packaging_syntax(request: &mut Dictionary) -> (bool, Vec<String>) {
    let mut errors = Vec::new();
    let mut valid = true;
    for key in [
        "pkgroot", "pkgdir", "pkgname", "pkgtype", "id", "version", "infofile", "chown", "scripts",
    ] {
        if !request.contains_key(key) {
            errors.push(format!("Request is missing key '{key}'"));
            valid = false;
        } else if if key == "chown" {
            request[key].as_array().is_none()
        } else {
            request[key].as_string().is_none()
        } {
            errors.push(format!(
                "Request key {key} is not of type <class '{}'>",
                if key == "chown" { "list" } else { "str" }
            ));
            valid = false;
        }
    }
    if valid {
        if request["pkgtype"].as_string() != Some("flat") {
            errors.push("pkgtype must be flat".into());
            valid = false;
        }
        for entry in request["chown"].as_array().unwrap() {
            let Some(entry) = entry.as_dictionary() else {
                errors.push("chown entry isn't dictionary".into());
                valid = false;
                continue;
            };
            for key in ["path", "user", "group", "mode"] {
                let Some(value) = entry.get(key) else {
                    // Reference records missing chown keys but leaves syntax_ok
                    // true; missing owner/group then fails in the worker.
                    errors.push(format!("chown entry is missing {key}"));
                    continue;
                };
                let numeric = matches!(key, "user" | "group");
                if value.as_string().is_none()
                    && !(numeric && matches!(value, Value::Integer(_) | Value::Boolean(_)))
                {
                    errors.push(format!(
                        "Request key chown.{key} is not of type {}",
                        if numeric {
                            "(<class 'str'>, <class 'int'>)"
                        } else {
                            "<class 'str'>"
                        }
                    ));
                    valid = false;
                }
            }
        }
        if !request.contains_key("pkgbuild_args") {
            request.insert("pkgbuild_args".into(), Vec::<Value>::new().into());
        }
        if let Some(args) = request["pkgbuild_args"].as_array() {
            for arg in args {
                if arg.as_string().is_none() {
                    errors.push(format!("pkgbuild_args entry is not a string: {arg:?}"));
                    valid = false;
                }
            }
        } else {
            errors.push("Request key pkgbuild_args is not of type <class 'list'>".into());
            valid = false;
        }
    }
    (valid, errors)
}
fn reply(stream: &mut UnixStream, text: &str) -> Result<(), String> {
    stream.write_all(text.as_bytes()).map_err(|e| e.to_string())
}
fn handle(stream: &mut UnixStream, service: Service, uid: u32, gid: u32) -> Result<(), String> {
    // The reference wire format is one plist in a single recv(8192), without a
    // length prefix or client half-close. Retain that framing for existing clients.
    let mut bytes = [0u8; 8192];
    let count = stream.read(&mut bytes).map_err(|e| e.to_string())?;
    let mut data = match Value::from_reader(std::io::Cursor::new(&bytes[..count])) {
        Ok(value) => value,
        Err(_) => return reply(stream, "ERROR:Malformed request\n"),
    };
    let Some(request) = data.as_dictionary_mut() else {
        return reply(
            stream,
            if matches!(service, Service::Packaging) {
                "ERROR:Request root is not a dictionary\n"
            } else {
                "ERROR:Unsupported request format"
            },
        );
    };
    match service {
        Service::Packaging => {
            let (valid, errors) = packaging_syntax(request);
            if !valid {
                return reply(
                    stream,
                    &errors
                        .iter()
                        .map(|e| format!("ERROR:{e}\n"))
                        .collect::<String>(),
                );
            }
            if !Path::new(request["pkgroot"].as_string().unwrap()).exists() {
                return reply(stream, "ERROR:Can't find pkgroot");
            }
            match packager::package(request, uid, gid) {
                Ok(path) => reply(stream, &format!("OK:{}\n", path.display())),
                Err(error) => reply(stream, &format!("{error}\n")),
            }
        }
        Service::Installation => {
            let result = if request.contains_key("package") {
                installer::install(request, stream)
            } else if request.contains_key("mount_point") {
                copier::copy(request, stream)
            } else {
                return reply(stream, "ERROR:Unsupported request format");
            };
            match result {
                Ok(()) => reply(stream, "OK:DONE\n"),
                Err(error) => reply(stream, &format!("ERROR:{error}\n")),
            }
        }
    }
}
fn verify_executable(path: &Path) -> Result<(), String> {
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    let mut path = resolved.as_path();
    let mut errors = Vec::new();
    loop {
        let info = fs::metadata(path).map_err(|e| e.to_string())?;
        if info.uid() != 0 {
            errors.push(format!("{} must be owned by root.", path.display()));
        }
        if !matches!(info.gid(), 0 | 80) {
            errors.push(format!(
                "{} must have group wheel or admin.",
                path.display()
            ));
        }
        if info.mode() & 0o002 != 0 {
            errors.push(format!("{} mustn't be world writeable.", path.display()));
        }
        let Some(parent) = path.parent() else { break };
        path = parent;
        if path == Path::new("/") {
            break;
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
#[cfg(target_os = "macos")]
fn peer(stream: &UnixStream) -> Result<(u32, u32), String> {
    use std::os::fd::AsRawFd;
    let mut credential: libc::xucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::xucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            0,
            libc::LOCAL_PEERCRED,
            (&mut credential as *mut libc::xucred).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if credential.cr_version != 0 || length as usize != std::mem::size_of::<libc::xucred>() {
        return Err("Incompatible struct xucred version".into());
    }
    if !(1..=16).contains(&credential.cr_ngroups) {
        return Err("Peer has no valid primary group".into());
    }
    Ok((credential.cr_uid, credential.cr_groups[0]))
}
#[cfg(target_os = "macos")]
fn activate(service: Service) -> Result<std::os::unix::net::UnixListener, String> {
    use std::{ffi::CString, os::fd::FromRawFd, ptr};
    extern "C" {
        fn launch_activate_socket(
            name: *const libc::c_char,
            fds: *mut *mut libc::c_int,
            count: *mut libc::size_t,
        ) -> libc::c_int;
    }
    let name = CString::new(service.name()).unwrap();
    let mut fds = ptr::null_mut();
    let mut count = 0;
    let error = unsafe { launch_activate_socket(name.as_ptr(), &mut fds, &mut count) };
    if error != 0 {
        if !fds.is_null() {
            unsafe { libc::free(fds.cast()) };
        }
        return Err(format!(
            "Failed to retrieve sockets from launchd: {}",
            std::io::Error::from_raw_os_error(error)
        ));
    }
    if fds.is_null() || count == 0 {
        if !fds.is_null() {
            unsafe { libc::free(fds.cast()) };
        }
        return Err("launchd returned no sockets".into());
    }
    // launchd transfers descriptors; copy the first into a Rust owner and release
    // its allocated array. No reference to the freed array escapes this block.
    let fd = unsafe {
        let fd = *fds;
        for index in 1..count {
            libc::close(*fds.add(index));
        }
        libc::free(fds.cast());
        fd
    };
    let listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(fd) };
    if unsafe { libc::listen(fd, 10) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(listener)
}
#[cfg(target_os = "macos")]
struct Log {
    path: std::path::PathBuf,
    file: fs::File,
}
#[cfg(target_os = "macos")]
impl Log {
    fn open(service: Service) -> Result<Self, String> {
        let path = std::path::PathBuf::from(format!("/private/var/log/{}", service.name()));
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("Can't open log: {e}"))?;
        Ok(Self { path, file })
    }
    fn write(&mut self, text: &str) {
        eprintln!("{text}");
        if self
            .file
            .metadata()
            .map(|m| m.len() >= 100_000)
            .unwrap_or(false)
        {
            for index in (1..=9).rev() {
                let target = self.path.with_file_name(format!(
                    "{}.{}",
                    self.path.file_name().unwrap().to_string_lossy(),
                    index
                ));
                let source = if index == 1 {
                    self.path.clone()
                } else {
                    self.path.with_file_name(format!(
                        "{}.{}",
                        self.path.file_name().unwrap().to_string_lossy(),
                        index - 1
                    ))
                };
                if source.exists() {
                    let _ = fs::rename(source, target);
                }
            }
            if let Ok(file) = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
            {
                self.file = file;
            }
        }
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(self.file, "{timestamp} [{}]: {text}", std::process::id());
    }
}
#[cfg(target_os = "macos")]
pub fn run(service: Service) -> Result<(), String> {
    use std::{
        os::fd::AsRawFd,
        thread,
        time::{Duration, Instant},
    };
    let startup = (|| {
        if unsafe { libc::geteuid() } != 0 {
            return Err(format!("{} must be run as root.", service.name()));
        }
        verify_executable(&std::env::current_exe().map_err(|e| e.to_string())?)?;
        activate(service)
    })();
    let listener = match startup {
        Ok(listener) => listener,
        Err(error) => {
            thread::sleep(Duration::from_secs(10));
            return Err(error);
        }
    };
    let started = Instant::now();
    let mut log = Log::open(service)?;
    log.write(&format!(
        "{} native development helper starting",
        service.name()
    ));
    loop {
        let mut descriptor = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut descriptor, 1, 10_000) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.to_string());
        }
        if ready == 0 {
            if started.elapsed() >= Duration::from_secs(10) {
                break;
            }
            continue;
        }
        let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
        let result = peer(&stream).and_then(|(uid, gid)| {
            log.write(&format!("Handling request from uid {uid} gid {gid}"));
            handle(&mut stream, service, uid, gid)
        });
        if let Err(error) = result {
            log.write(&format!("Caught exception: {error}"));
            let _ = reply(&mut stream, &format!("ERROR:Caught exception: {error}"));
        }
    }
    log.write("Idle timeout; helper exiting");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, thread};
    fn exchange(service: Service, bytes: Vec<u8>) -> String {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || handle(&mut server, service, 501, 20).unwrap());
        client.write_all(&bytes).unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        worker.join().unwrap();
        reply
    }
    fn plist(value: Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn isolated_socket_replies_preserve_protocol_errors() {
        assert_eq!(
            exchange(Service::Packaging, b"invalid".to_vec()),
            "ERROR:Malformed request\n"
        );
        assert_eq!(
            exchange(Service::Packaging, plist(Value::Array(vec![]))),
            "ERROR:Request root is not a dictionary\n"
        );
        assert_eq!(
            exchange(Service::Installation, plist(Dictionary::new().into())),
            "ERROR:Unsupported request format"
        );
        assert_eq!(
            exchange(
                Service::Installation,
                plist(
                    Dictionary::from_iter([("package", Value::String("/not/a/package".into()))])
                        .into()
                )
            ),
            "ERROR:ERROR:No recipe_cache_dir in request\n"
        );
    }
    #[test]
    fn packaging_schema_checks_all_root_fields_and_defaults_arguments() {
        let (valid, errors) = packaging_syntax(&mut Dictionary::new());
        assert!(!valid);
        assert_eq!(errors.len(), 9);
        let mut request = Dictionary::new();
        for key in [
            "pkgroot", "pkgdir", "pkgname", "id", "version", "infofile", "scripts",
        ] {
            request.insert(key.into(), "".into());
        }
        request.insert("pkgtype".into(), "flat".into());
        request.insert("chown".into(), Vec::<Value>::new().into());
        assert!(packaging_syntax(&mut request).0);
        assert_eq!(request["pkgbuild_args"].as_array().unwrap().len(), 0);
    }
    #[test]
    fn startup_rejects_world_writable_executable() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("helper");
        fs::write(&path, "test").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(verify_executable(&path)
            .unwrap_err()
            .contains("world writeable"));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn peer_credentials_come_from_kernel() {
        let (client, _) = UnixStream::pair().unwrap();
        let (uid, _) = peer(&client).unwrap();
        assert_eq!(uid, unsafe { libc::geteuid() });
    }
}
