use super::{io, remove, string, truth, Result};
use plist::{Dictionary, Value};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::Command,
};
fn format(path: &str) -> Option<&'static str> {
    [
        ("zip", &["zip"][..]),
        ("tar_gzip", &["tar.gz", "tgz"][..]),
        ("tar_bzip2", &["tar.bz2", "tbz"][..]),
        ("tar", &["tar", "tar.xz"][..]),
        ("gzip", &["gzip"][..]),
    ]
    .into_iter()
    .find_map(|(kind, extensions)| extensions.iter().any(|e| path.ends_with(e)).then_some(kind))
}
fn validate(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut clean = PathBuf::new();
    for part in relative.components() {
        match part {
            Component::Normal(s) => clean.push(s),
            Component::CurDir => (),
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(format!(
                        "Archive contains path '{}' outside destination",
                        relative.display()
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "Archive contains absolute path '{}'",
                    relative.display()
                ))
            }
        }
    }
    let destination = root.join(clean);
    let realroot = io(root.canonicalize())?;
    for ancestor in destination.ancestors() {
        if ancestor.exists() {
            if !io(ancestor.canonicalize())?.starts_with(&realroot) {
                return Err(format!(
                    "Archive path '{}' resolves outside destination",
                    relative.display()
                ));
            }
            break;
        }
    }
    Ok(destination)
}
fn tar_reader(source: &Path) -> Result<Box<dyn Read>> {
    let mut file = io(fs::File::open(source))?;
    let mut magic = [0; 6];
    let count = io(file.read(&mut magic))?;
    drop(file);
    let file = io(fs::File::open(source))?;
    if count >= 2 && magic[..2] == [0x1f, 0x8b] {
        Ok(Box::new(flate2::read::GzDecoder::new(file)))
    } else if count >= 3 && &magic[..3] == b"BZh" {
        Ok(Box::new(bzip2::read::BzDecoder::new(file)))
    } else if count == 6 && magic == [0xfd, b'7', b'z', b'X', b'Z', 0] {
        Ok(Box::new(xz2::read::XzDecoder::new(file)))
    } else {
        Ok(Box::new(file))
    }
}
fn native(kind: &str, source: &Path, destination: &Path) -> Result<()> {
    if kind == "zip" {
        let mut archive =
            zip::ZipArchive::new(io(fs::File::open(source))?).map_err(|e| e.to_string())?;
        for i in 0..archive.len() {
            let item = archive.by_index(i).map_err(|e| e.to_string())?;
            validate(destination, Path::new(item.name()))?;
        }
        for i in 0..archive.len() {
            let mut item = archive.by_index(i).map_err(|e| e.to_string())?;
            let path = validate(destination, Path::new(item.name()))?;
            if item.is_dir() {
                io(fs::create_dir_all(path))?;
            } else {
                if let Some(parent) = path.parent() {
                    io(fs::create_dir_all(parent))?;
                }
                let mut file = io(fs::File::create(path))?;
                io(std::io::copy(&mut item, &mut file))?;
            }
        }
        Ok(())
    } else if kind.starts_with("tar") {
        let mut archive = tar::Archive::new(tar_reader(source)?);
        for entry in io(archive.entries())? {
            let entry = io(entry)?;
            let path = io(entry.path())?;
            validate(destination, &path)?;
            if let Some(link) = io(entry.link_name())? {
                let target = if entry.header().entry_type().is_symlink() {
                    path.parent().unwrap_or(Path::new("")).join(link)
                } else {
                    link.into_owned()
                };
                validate(destination, &target)?;
            }
        }
        let mut archive = tar::Archive::new(tar_reader(source)?);
        io(archive.unpack(destination))
    } else {
        Err(format!(
            "Native extraction does not support archive format {kind}"
        ))
    }
}
pub(super) fn execute(env: &mut Dictionary) -> Result<()> {
    let source = env
        .get("archive_path")
        .or_else(|| env.get("pathname"))
        .and_then(Value::as_string)
        .filter(|s| !s.is_empty())
        .ok_or("Expected an 'archive_path' input variable but none is set!")?
        .to_string();
    let destination = match env.get("destination_path").and_then(Value::as_string) {
        Some(path) => PathBuf::from(path),
        None => Path::new(string(env, "RECIPE_CACHE_DIR")?).join(string(env, "NAME")?),
    };
    if !destination.exists() {
        io(fs::create_dir_all(&destination))?;
    } else if truth(env.get("purge_destination")) {
        for entry in io(fs::read_dir(&destination))? {
            remove(&io(entry)?.path())?;
        }
    }
    let kind = env
        .get("archive_format")
        .and_then(Value::as_string)
        .or_else(|| format(&source))
        .ok_or_else(|| format!("Can't guess archive format for filename {source}"))?;
    if !truth(env.get("archive_format")) {
        autopkg_platform::processor_output(
            1,
            format!(
                "Guessed archive format '{kind}' from filename {}",
                Path::new(&source)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ),
        );
    }
    if !["zip", "tar_gzip", "tar_bzip2", "tar", "gzip"].contains(&kind) {
        return Err(format!("'{kind}' is not valid for archive_format"));
    }
    if truth(env.get("USE_PYTHON_NATIVE_EXTRACTOR")) {
        native(kind, Path::new(&source), &destination)?;
    } else {
        if !cfg!(target_os = "macos") {
            return Err("Platform archive utilities are only implemented on macOS; set USE_PYTHON_NATIVE_EXTRACTOR=true for the native Rust extractor".into());
        }
        let mut command = if kind == "zip" || kind == "gzip" {
            let mut c = Command::new("/usr/bin/ditto");
            c.args(["--noqtn", "-x"]);
            if kind == "zip" {
                c.arg("-k");
            }
            c.arg(&source).arg(&destination);
            c
        } else {
            let mut c = Command::new("/usr/bin/tar");
            c.args(["-x", "-f", &source, "-C"]).arg(&destination);
            if kind == "tar_gzip" {
                c.arg("-z");
            } else if kind == "tar_bzip2" {
                c.arg("-j");
            }
            c
        };
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "Unarchiving {source} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    autopkg_platform::processor_output(
        1,
        format!("Unarchived {source} to {}", destination.display()),
    );
    if truth(env.get("archive_format")) {
        env.remove("archive_format");
    }
    Ok(())
}
pub(super) fn zip_plist(path: &str, skip_root: bool) -> Result<Option<Dictionary>> {
    let normalized = path.replace('\\', "/");
    let lowercase = normalized.to_lowercase();
    let index = lowercase
        .find(".zip/")
        .ok_or("Expected a path inside a ZIP archive")?;
    let (archive_path, inner) = normalized.split_at(index + 4);
    let display_archive = super::normalized_path(archive_path);
    let display_archive = display_archive.display();
    let mut inner = inner[1..].to_string();
    let mut archive =
        zip::ZipArchive::new(io(fs::File::open(archive_path))?).map_err(|e| e.to_string())?;
    let mut roots = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().trim_end_matches('/');
        if entry.is_dir() && !name.contains('/') {
            roots.push(name.to_string());
        }
    }
    if roots.is_empty() {
        autopkg_platform::processor_output(1, format!("Zip archive '{display_archive}' is empty."));
        return Ok(None);
    }
    if skip_root {
        if roots.len() > 1 {
            return Err("Zip archive has more than one directory at root and skip_single_root_directory was set".into());
        }
        inner = format!("{}/{inner}", roots[0]);
    }
    let mut file = match archive.by_name(&inner) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) if skip_root => {
            autopkg_platform::processor_output(
                1,
                format!("Zip archive '{display_archive}' does not contain '{inner}'"),
            );
            return Ok(None);
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    io(file.read_to_end(&mut bytes))?;
    let plist = Value::from_reader(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    Ok(Some(
        plist
            .into_dictionary()
            .ok_or("ZIP member plist is not a dictionary")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{env, Temp};
    use std::io::Write;
    #[test]
    fn zip_native_and_traversal_preflight() {
        let t = Temp::new();
        let source = t.path("fixture.zip");
        let file = fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("folder/é.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"hello").unwrap();
        zip.finish().unwrap();
        let mut e = env(&[
            ("archive_path", &source),
            ("destination_path", &t.path("out")),
            ("archive_format", "zip"),
        ]);
        e.insert("USE_PYTHON_NATIVE_EXTRACTOR".into(), true.into());
        super::super::execute("Unarchiver", &mut e).unwrap();
        assert_eq!(fs::read(t.path("out/folder/é.txt")).unwrap(), b"hello");
        assert!(!e.contains_key("archive_format"));
        let file = fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"bad").unwrap();
        zip.finish().unwrap();
        assert!(super::super::execute("Unarchiver", &mut e).is_err());
        assert!(!t.0.join("escape").exists());
    }
    #[test]
    fn gzip_tar_native() {
        let t = Temp::new();
        let source = t.path("fixture.tar.gz");
        let file = fs::File::create(&source).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o640);
        header.set_cksum();
        tar.append_data(&mut header, "file.txt", &b"data"[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let mut e = env(&[
            ("archive_path", &source),
            ("destination_path", &t.path("out")),
        ]);
        e.insert("USE_PYTHON_NATIVE_EXTRACTOR".into(), true.into());
        super::super::execute("Unarchiver", &mut e).unwrap();
        assert_eq!(fs::read(t.path("out/file.txt")).unwrap(), b"data");
    }
}

#[cfg(test)]
#[test]
fn versioner_zip_root_selection() {
    use std::io::Write;
    let temp = crate::tests::Temp::new();
    let path = temp.path("app.zip");
    let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    archive
        .add_directory("release/", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive
        .start_file(
            "release/Info.plist",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    let mut data = Dictionary::new();
    data.insert("CFBundleShortVersionString".into(), "4.2".into());
    let mut bytes = Vec::new();
    Value::Dictionary(data)
        .to_writer_binary(&mut bytes)
        .unwrap();
    archive.write_all(&bytes).unwrap();
    archive.finish().unwrap();
    let mut env = crate::tests::env(&[("input_plist_path", &format!("{path}/Info.plist"))]);
    env.insert("skip_single_root_dir".into(), true.into());
    crate::execute("Versioner", &mut env).unwrap();
    assert_eq!(env["version"].as_string(), Some("4.2"));
    env.insert("skip_single_root_dir".into(), false.into());
    assert!(crate::execute("Versioner", &mut env).is_err());
}
