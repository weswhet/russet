//! The pinned Python downloader adds certifi to OpenSSL's selected default
//! locations. On Windows Python additionally imports the CA and ROOT stores.
use crate::ExecutionFailure;
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const CERTIFI: &[u8] = include_bytes!("../data/certifi/cacert.pem");

// AutoPkg's shipped macOS Python sitecustomize replaces absent, empty, and
// non-file overrides with certifi. Preserve that policy without changing env.
fn custom_file(path: Option<OsString>) -> Option<PathBuf> {
    let path = path.map(PathBuf::from);
    #[cfg(target_os = "macos")]
    return path.filter(|p| p.is_file());
    #[cfg(not(target_os = "macos"))]
    path
}

pub(super) fn explicit_certificate() -> Option<PathBuf> {
    custom_file(std::env::var_os("SSL_CERT_FILE"))
}

fn selected_file(custom: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = custom {
        return Some(path.to_owned());
    }
    #[cfg(target_os = "macos")]
    return None; // The embedded certifi bundle supplies the shipped default.
    #[cfg(target_os = "linux")]
    return Some("/usr/lib/ssl/cert.pem".into());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    None
}

pub(super) fn capath() -> Option<OsString> {
    // Schannel has no CApath support; bundle() resolves its hashed directories.
    #[cfg(not(windows))]
    if let Some(path) = std::env::var_os("SSL_CERT_DIR") {
        return Some(path);
    }
    #[cfg(target_os = "macos")]
    return Some("/Library/Frameworks/Python.framework/Versions/3.11/etc/openssl/certs".into());
    #[cfg(target_os = "linux")]
    return Some("/usr/lib/ssl/certs".into());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    None
}

/// The pinned OpenSSL accepts matching numbered hash entries even when a
/// previous index is absent. Some curl backends require consecutive indexes.
fn hashed_directory(path: &Path, gaps_only: bool) -> Result<Vec<u8>, ExecutionFailure> {
    let Ok(entries) = fs::read_dir(path) else {
        return Ok(Vec::new());
    };
    let mut pem = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some((hash, index)) = name.split_once('.') else {
            continue;
        };
        if hash.len() != 8
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            continue;
        }
        if index.is_empty() || !index.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(index) = index.parse::<u32>() else {
            continue;
        };
        if gaps_only && (0..index).all(|n| path.join(format!("{hash}.{n}")).is_file()) {
            continue;
        }
        let file = entry.path();
        let result = std::process::Command::new("openssl")
            .args(["x509", "-subject_hash", "-noout", "-in"])
            .arg(&file)
            .output()
            .map_err(|e| {
                ExecutionFailure::unexpected(format!(
                    "This SSL_CERT_DIR layout requires the OpenSSL executable: {e}"
                ))
            })?;
        if result.status.success() && String::from_utf8_lossy(&result.stdout).trim() == hash {
            if let Ok(bytes) = fs::read(&file) {
                if let Ok(bytes) = validated(&bytes) {
                    pem.extend(bytes);
                    pem.push(b'\n');
                }
            }
        }
    }
    Ok(pem)
}

fn validated(bytes: &[u8]) -> Result<Vec<u8>, ExecutionFailure> {
    let certs = rustls_pemfile::certs(&mut &*bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ExecutionFailure::unexpected(format!("SSL certificate PEM: {e}")))?;
    if certs.is_empty() {
        return Err(ExecutionFailure::unexpected(
            "SSL certificate PEM contains no certificates",
        ));
    }
    let mut roots = rustls::RootCertStore::empty();
    for cert in certs {
        roots
            .add(cert)
            .map_err(|e| ExecutionFailure::unexpected(format!("SSL certificate PEM: {e}")))?;
    }
    Ok(bytes.to_vec())
}

fn custom(path: &Path) -> Result<Vec<u8>, ExecutionFailure> {
    if !path.is_file() {
        return Err(format!("Certificate file '{}' does not exist.", path.display()).into());
    }
    let bytes = fs::read(path)
        .map_err(|_| format!("Certificate file '{}' is not readable.", path.display()))?;
    validated(&bytes)
}

fn combined(default: Option<&[u8]>, native: &[u8], custom: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for part in [default, Some(native), Some(CERTIFI), custom]
        .into_iter()
        .flatten()
    {
        bytes.extend_from_slice(part);
        bytes.push(b'\n');
    }
    bytes
}

/// Schannel curl limits its CA file to 1 MiB. Native CA/ROOT stores and
/// certifi overlap, so encode their set union rather than repeated PEM copies.
#[cfg(any(windows, test))]
fn compact_certificates(bytes: &[u8]) -> Result<Vec<u8>, ExecutionFailure> {
    use base64::Engine;
    let mut seen = std::collections::BTreeSet::new();
    let mut pem = Vec::new();
    for cert in rustls_pemfile::certs(&mut &*bytes) {
        let cert =
            cert.map_err(|e| ExecutionFailure::unexpected(format!("SSL certificate PEM: {e}")))?;
        if !seen.insert(cert.as_ref().to_vec()) {
            continue;
        }
        pem.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
        let encoded = base64::engine::general_purpose::STANDARD.encode(cert.as_ref());
        for line in encoded.as_bytes().chunks(64) {
            pem.extend_from_slice(line);
            pem.push(b'\n');
        }
        pem.extend_from_slice(b"-----END CERTIFICATE-----\n");
    }
    Ok(pem)
}

#[cfg(windows)]
fn native() -> Vec<u8> {
    use base64::Engine;
    use schannel::{cert_context::ValidUses, cert_store::CertStore};
    let mut bytes = Vec::new();
    for name in ["CA", "ROOT"] {
        let Ok(store) = CertStore::open_current_user(name) else {
            continue;
        };
        for cert in store.certs() {
            let permitted = match cert.valid_uses() {
                Ok(ValidUses::All) => true,
                Ok(ValidUses::Oids(oids)) => oids.iter().any(|oid| oid == "1.3.6.1.5.5.7.3.1"),
                Err(_) => false,
            };
            if permitted {
                bytes.extend_from_slice(b"-----BEGIN CERTIFICATE-----\n");
                let encoded = base64::engine::general_purpose::STANDARD.encode(cert.to_der());
                for line in encoded.as_bytes().chunks(64) {
                    bytes.extend_from_slice(line);
                    bytes.push(b'\n');
                }
                bytes.extend_from_slice(b"-----END CERTIFICATE-----\n");
            }
        }
    }
    bytes
}
#[cfg(not(windows))]
fn native() -> Vec<u8> {
    Vec::new()
}

pub(crate) struct Bundle {
    file: tempfile::TempPath,
    pub capath: Option<OsString>,
    _empty_directory: Option<tempfile::TempDir>,
    pub implicit_certificate: Option<tempfile::TempPath>,
}

/// Emulate the shipped interpreter's sitecustomize for this child only.
/// Keep native curl's own environment/option precedence and trust backend.
pub(crate) fn native_curl(
    command: &mut std::process::Command,
) -> Result<Option<Bundle>, ExecutionFailure> {
    native_curl_for(command, std::env::var_os("SSL_CERT_FILE"))
}

fn native_curl_for(
    command: &mut std::process::Command,
    certificate: Option<OsString>,
) -> Result<Option<Bundle>, ExecutionFailure> {
    if !cfg!(target_os = "macos") || custom_file(certificate).is_some() {
        return Ok(None);
    }
    let mut file = tempfile::Builder::new()
        .prefix("autopkg-certifi-")
        .suffix(".pem")
        .tempfile()
        .map_err(|e| e.to_string())?;
    file.write_all(CERTIFI).map_err(|e| e.to_string())?;
    let file = file.into_temp_path();
    command.env("SSL_CERT_FILE", &file);
    Ok(Some(Bundle {
        file,
        capath: None,
        _empty_directory: None,
        implicit_certificate: None,
    }))
}
impl Bundle {
    pub fn path(&self) -> &Path {
        &self.file
    }
}

pub(super) fn bundle() -> Result<Bundle, ExecutionFailure> {
    let custom_path = explicit_certificate();
    let capath = capath();
    let directories = std::env::var_os("SSL_CERT_DIR").or_else(|| capath.clone());
    bundle_for(custom_path.as_deref(), capath, directories)
}

fn bundle_for(
    custom_path: Option<&Path>,
    mut capath: Option<OsString>,
    directories: Option<OsString>,
) -> Result<Bundle, ExecutionFailure> {
    let custom = custom_path.map(custom).transpose()?;
    let default = selected_file(custom_path).and_then(|p| fs::read(p).ok());
    let mut native = native();
    if let Some(paths) = directories {
        for path in std::env::split_paths(&paths) {
            if !path.as_os_str().is_empty() {
                native.extend(hashed_directory(&path, !cfg!(windows))?);
            }
        }
    }
    let empty_directory = if capath.as_ref().is_some_and(|p| p.is_empty()) {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        capath = Some(directory.path().as_os_str().to_owned());
        Some(directory)
    } else {
        None
    };
    let bytes = combined(default.as_deref(), &native, custom.as_deref());
    let implicit_certificate = if cfg!(target_os = "macos") && custom_path.is_none() {
        let mut file = tempfile::Builder::new()
            .prefix("autopkg-certifi-")
            .suffix(".pem")
            .tempfile()
            .map_err(|e| e.to_string())?;
        file.write_all(CERTIFI).map_err(|e| e.to_string())?;
        Some(file.into_temp_path())
    } else {
        None
    };
    #[cfg(windows)]
    let bytes = compact_certificates(&bytes)?;
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    Ok(Bundle {
        // Schannel opens CA files without sharing a pre-existing write handle.
        // Close ours before starting curl, retaining RAII deletion of the path.
        file: file.into_temp_path(),
        capath,
        _empty_directory: empty_directory,
        implicit_certificate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    #[cfg(target_os = "macos")]
    #[test]
    fn native_curl_fallback_is_scoped_and_preserves_valid_overrides() {
        let root = tempfile::tempdir().unwrap();
        for value in [
            None,
            Some(OsString::new()),
            Some(root.path().as_os_str().to_owned()),
            Some(root.path().join("missing.pem").into_os_string()),
        ] {
            let mut command = std::process::Command::new("curl");
            let bundle = native_curl_for(&mut command, value).unwrap().unwrap();
            assert_eq!(fs::read(bundle.path()).unwrap(), CERTIFI);
            let changed = command.get_envs().collect::<Vec<_>>();
            assert_eq!(
                changed,
                vec![(
                    std::ffi::OsStr::new("SSL_CERT_FILE"),
                    Some(bundle.path().as_os_str())
                )]
            );
            let path = bundle.path().to_owned();
            drop(bundle);
            assert!(!path.exists());
        }
        let valid = root.path().join("existing.pem");
        // sitecustomize checks file existence, not PEM validity.
        fs::write(&valid, b"").unwrap();
        let mut command = std::process::Command::new("curl");
        assert!(native_curl_for(&mut command, Some(valid.into_os_string()))
            .unwrap()
            .is_none());
        assert_eq!(command.get_envs().count(), 0);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn shipped_macos_defaults_are_certifi_and_invalid_paths_fall_back() {
        let bundle = bundle_for(None, Some(OsString::new()), None).unwrap();
        let resource = bundle.implicit_certificate.as_ref().unwrap();
        assert_eq!(fs::read(resource).unwrap(), CERTIFI);
        assert_eq!(
            compact_certificates(&fs::read(bundle.path()).unwrap()).unwrap(),
            compact_certificates(CERTIFI).unwrap()
        );
        for path in [
            None,
            Some(OsString::new()),
            Some("/nonexistent-autopkg-ca.pem".into()),
        ] {
            let selected = custom_file(path);
            assert!(selected.is_none());
            assert!(selected_file(selected.as_deref()).is_none());
            let bytes = combined(None, &[], None);
            assert_eq!(
                compact_certificates(&bytes).unwrap(),
                compact_certificates(CERTIFI).unwrap()
            );
            assert_eq!(rustls_pemfile::certs(&mut bytes.as_slice()).count(), 147);
        }
        let root = tempfile::tempdir().unwrap();
        assert!(custom_file(Some(root.path().as_os_str().to_owned())).is_none());
        let file = root.path().join("empty.pem");
        fs::write(&file, b"").unwrap();
        assert_eq!(
            custom_file(Some(file.as_os_str().to_owned())),
            Some(file.clone())
        );
        assert!(custom(&file).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_default_file_is_independent_of_capath() {
        // setup-python's selected release image uses these OpenSSL defaults.
        // An empty SSL_CERT_DIR must not remove the selected default CA file.
        assert_eq!(
            selected_file(None),
            Some(PathBuf::from("/usr/lib/ssl/cert.pem"))
        );
        let default = fs::read(selected_file(None).unwrap()).unwrap();
        let bundle = bundle_for(None, Some(OsString::new()), None).unwrap();
        assert!(bundle.capath.as_ref().is_some_and(|p| !p.is_empty()));
        let actual = fs::read(bundle.path()).unwrap();
        assert_eq!(
            compact_certificates(&actual).unwrap(),
            compact_certificates(&combined(Some(&default), &[], None)).unwrap()
        );
        // The selected image's system file contributes roots beyond certifi;
        // this catches accidentally dropping the file when CApath is empty.
        assert!(
            rustls_pemfile::certs(&mut compact_certificates(&actual).unwrap().as_slice()).count()
                > 147
        );
    }
    #[test]
    fn schannel_bundle_compaction_preserves_unique_roots() {
        let mut duplicated = CERTIFI.repeat(5);
        duplicated.extend_from_slice(include_bytes!("../data/trust-fixtures/custom.pem"));
        assert!(duplicated.len() > 1_048_576);
        let compact = compact_certificates(&duplicated).unwrap();
        assert!(compact.len() < 1_048_576);
        let certificates = |bytes: &[u8]| {
            rustls_pemfile::certs(&mut &*bytes)
                .map(|cert| cert.unwrap().as_ref().to_vec())
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(certificates(&compact), certificates(&duplicated));
        assert_eq!(rustls_pemfile::certs(&mut compact.as_slice()).count(), 148);
    }
    #[cfg(windows)]
    #[test]
    fn bundle_releases_write_handle_before_schannel_reads() {
        use std::os::windows::fs::OpenOptionsExt;
        let bundle = bundle().unwrap();
        let size = fs::metadata(bundle.path()).unwrap().len();
        assert!(
            size <= 1_048_576,
            "Native Schannel CA bundle exceeds curl's 1 MiB limit: {size} bytes"
        );
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(bundle.path())
            .unwrap();
    }
    #[test]
    fn pinned_certifi_bundle_and_additive_roots() {
        assert_eq!(
            format!("{:x}", Sha256::digest(CERTIFI)),
            "2089fc5a25836401faee99f722460b01b393999746a0fb817943666d6ecc0458"
        );
        let mut pem = CERTIFI;
        assert_eq!(rustls_pemfile::certs(&mut pem).count(), 147);
        validated(CERTIFI).unwrap();
        let default = include_bytes!("../data/trust-fixtures/default.pem");
        let custom = include_bytes!("../data/trust-fixtures/custom.pem");
        let bytes = combined(Some(default), &[], Some(custom));
        validated(&bytes).unwrap();
        let fingerprints = |bytes: &[u8]| {
            rustls_pemfile::certs(&mut &*bytes)
                .map(|cert| format!("{:x}", Sha256::digest(cert.unwrap().as_ref())))
                .collect::<std::collections::BTreeSet<_>>()
        };
        let roots = fingerprints(&bytes);
        assert_eq!(roots.len(), 149);
        for source in [CERTIFI, default.as_slice(), custom.as_slice()] {
            assert!(fingerprints(source).is_subset(&roots));
        }
        // An explicit POSIX cafile replaces the default cafile. It does not
        // replace certifi, and it must not accidentally keep the default root.
        let replaced = fingerprints(&combined(None, &[], Some(custom)));
        assert_eq!(replaced.len(), 148);
        assert!(fingerprints(default).is_disjoint(&replaced));
    }
    #[test]
    fn malformed_custom_certificates_fail_before_transfer() {
        assert!(validated(b"not a certificate").is_err());
        assert!(
            validated(b"-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----\n").is_err()
        );
        assert!(custom(Path::new("/nonexistent-autopkg-certificate.pem")).is_err());
    }
    #[test]
    #[ignore = "requires the OpenSSL executable used for Windows CApath imports"]
    fn hashed_directory_ignores_unindexed_wrong_hash_and_accepts_gaps() {
        let root = tempfile::tempdir().unwrap();
        let pem = include_bytes!("../data/trust-fixtures/custom.pem");
        let plain = root.path().join("arbitrary.pem");
        fs::write(&plain, pem).unwrap();
        assert!(hashed_directory(root.path(), false).unwrap().is_empty());
        let output = std::process::Command::new("openssl")
            .args(["x509", "-subject_hash", "-noout", "-in"])
            .arg(&plain)
            .output()
            .unwrap();
        assert!(output.status.success());
        let hash = String::from_utf8(output.stdout).unwrap();
        let hash = hash.trim();
        fs::write(root.path().join("00000000.0"), pem).unwrap();
        fs::write(root.path().join(format!("{hash}.+1")), pem).unwrap();
        assert!(hashed_directory(root.path(), false).unwrap().is_empty());
        fs::write(root.path().join(format!("{hash}.1")), pem).unwrap();
        assert!(!hashed_directory(root.path(), false).unwrap().is_empty());
        fs::write(root.path().join(format!("{hash}.0")), pem).unwrap();
        let bytes = hashed_directory(root.path(), false).unwrap();
        assert_eq!(rustls_pemfile::certs(&mut bytes.as_slice()).count(), 2);
    }
}
