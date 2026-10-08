//! Native replacement for the `pkgutil` operations Russet uses on flat
//! packages: [`expand`] (`pkgutil --expand`), [`flatten`]
//! (`pkgutil --flatten`), and [`check_signature`]
//! (`pkgutil --check-signature`).
#![forbid(unsafe_code)]

#[cfg(unix)]
pub use imp::*;

#[cfg(all(test, unix))]
mod tests;

#[cfg(unix)]
mod imp {
    use russet_fs::Limits;
    use russet_xar::{Archive, Builder, Content, Encoding, EntryKind};
    use std::fs;
    use std::io::{self, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    pub use russet_codesign::package::PackageSignature;

    /// Checks a flat package's signature like
    /// `pkgutil --check-signature package`, at time `now`. Fails when the
    /// package is unsigned or the signature, its timestamp, or its chain to an
    /// Apple root doesn't verify.
    pub fn check_signature(
        package: &Path,
        now: std::time::SystemTime,
    ) -> Result<PackageSignature, String> {
        let archive = Archive::open(package).map_err(|e| e.to_string())?;
        let (algorithm, checksum) = archive.checksum().ok_or("Status: no signature")?;
        let rsa = archive
            .signatures()
            .iter()
            .find(|s| s.style == "RSA")
            .ok_or("Status: no signature")?;
        let cms = archive.signatures().iter().find(|s| s.style == "CMS");
        let style = match algorithm {
            russet_xar::Algorithm::Sha1 => "sha1",
            russet_xar::Algorithm::Sha256 => "sha256",
            russet_xar::Algorithm::Sha512 => "sha512",
            russet_xar::Algorithm::Md5 => {
                return Err("Packages signed over an MD5 checksum aren't trusted".into())
            }
        };
        russet_codesign::package::verify(
            &russet_codesign::package::SignedPackage {
                checksum_algorithm: russet_codesign::package::checksum_algorithm(style).unwrap(),
                checksum,
                rsa_signature: &rsa.bytes,
                rsa_certificates: &rsa.certificates,
                cms_signature: cms.map(|s| s.bytes.as_slice()),
            },
            now,
        )
    }

    /// Members that `pkgutil --flatten` compresses with bzip2.
    const COMPRESSED: [&str; 3] = ["Bom", "PackageInfo", "Distribution"];
    /// Archives inside a package that are folders when expanded.
    const ARCHIVES: [&str; 2] = ["Scripts", "Payload"];

    fn error(path: &Path, verb: &str, e: io::Error) -> io::Error {
        io::Error::new(
            e.kind(),
            format!("Error encountered while {verb} {}. {e}", path.display()),
        )
    }

    /// Expands a flat package like `pkgutil --expand package destination`.
    ///
    /// `destination` must not exist, and its parent must. Each component's
    /// `Scripts` archive becomes a folder, with `._name` members kept as
    /// ordinary files; `Payload` stays a compressed archive.
    pub fn expand(package: &Path, destination: &Path, limits: Limits) -> io::Result<()> {
        fs::create_dir(destination).map_err(|e| error(destination, "creating", e))?;
        let mut archive = Archive::open(package).map_err(|e| error(package, "reading", e))?;
        archive.extract(destination, limits, |_| false)?;
        let scripts: Vec<PathBuf> = archive
            .entries()
            .iter()
            .filter(|e| {
                e.kind == EntryKind::File && e.path.file_name().is_some_and(|n| n == "Scripts")
            })
            .map(|e| destination.join(&e.path))
            .collect();
        for archive in scripts {
            // Unpack into a folder this call creates, never a path the
            // package could have put a symlink at.
            let parent = archive.parent().unwrap_or(destination);
            let private = tempfile::Builder::new()
                .prefix(".Scripts.")
                .tempdir_in(parent)?;
            let unpacked = private.path().join("Scripts");
            russet_ditto::extract_cpio_with(&archive, &unpacked, limits, false)?;
            fs::remove_file(&archive)?;
            fs::rename(&unpacked, &archive)?;
        }
        Ok(())
    }

    /// Flattens an expanded package folder like
    /// `pkgutil --flatten folder package`, replacing `package` if it exists.
    ///
    /// `Scripts` and `Payload` folders are archived as gzip-compressed cpio;
    /// `Bom`, `PackageInfo`, and `Distribution` are stored with bzip2.
    pub fn flatten(folder: &Path, package: &Path) -> io::Result<()> {
        let parent = package
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let scratch = tempfile::tempdir_in(parent)?;
        let mut builder = Builder::new();
        add(folder, Path::new(""), &mut builder, scratch.path())?;
        let staged = scratch.path().join("package");
        builder.write(&staged)?;
        fs::rename(&staged, package).map_err(|e| error(package, "writing", e))
    }

    fn add(root: &Path, relative: &Path, builder: &mut Builder, scratch: &Path) -> io::Result<()> {
        let mut children: Vec<_> = fs::read_dir(root.join(relative))?
            .map(|e| e.map(|e| e.file_name()))
            .collect::<io::Result<_>>()?;
        children.sort();
        for name in children {
            let child = relative.join(&name);
            let path = root.join(&child);
            let metadata = fs::symlink_metadata(&path)?;
            let mode = metadata.permissions().mode() & 0o7777;
            let name = name.to_string_lossy();
            if metadata.is_dir() && ARCHIVES.contains(&name.as_ref()) {
                // A unique file per archive: names built from the path can
                // collide (`a.pkg/b.pkg` and `a.pkg-b.pkg`).
                let (file, archive) = tempfile::Builder::new()
                    .prefix("archive-")
                    .tempfile_in(scratch)?
                    .keep()
                    .map_err(|e| e.error)?;
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut encoder =
                    russet_ditto::write_tree(&path, encoder, |_, m| russet_ditto::Header {
                        mode: m.mode(),
                        uid: m.uid(),
                        gid: m.gid(),
                        mtime: m.mtime().max(0) as u64,
                        ino: 0,
                        nlink: m.nlink() as u32,
                    })?
                    .finish()?;
                encoder.flush()?;
                builder.add_file(&child, 0o644, Content::Path(archive), Encoding::None)?;
            } else if metadata.is_dir() {
                builder.add_directory(&child, mode)?;
                add(root, &child, builder, scratch)?;
            } else if metadata.is_file() {
                let encoding = if COMPRESSED.contains(&name.as_ref()) {
                    Encoding::Bzip2
                } else {
                    Encoding::None
                };
                builder.add_file(&child, mode, Content::Path(path), encoding)?;
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} isn't a file or folder, so it can't be flattened",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}
