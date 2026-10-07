//! Scoped native disk-image mounts shared by processors and metadata inspection.
use plist::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
type Result<T> = std::result::Result<T, String>;
struct MountDirectory(PathBuf);
impl MountDirectory {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for MountDirectory {
    fn drop(&mut self) {
        // A failed attach/detach may leave a live filesystem here. Never use
        // recursive deletion on a directory used as a mount root.
        if let Ok(entries) = std::fs::read_dir(&self.0) {
            for entry in entries.flatten() {
                #[cfg(target_os = "macos")]
                if std::fs::remove_dir(entry.path()).is_err() {
                    // Failed attach output can still leave a mounted volume.
                    // Only inspect this guard's private mount directory.
                    let _ = Command::new("/usr/bin/hdiutil")
                        .arg("detach")
                        .arg(entry.path())
                        .output();
                }
                let _ = std::fs::remove_dir(entry.path());
            }
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|e| e.to_string())
}
fn mac() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("Disk image operations are only supported on macOS".into())
    }
}
pub fn parse_hdiutil_plist(bytes: &[u8]) -> Result<Value> {
    let text = String::from_utf8_lossy(bytes);
    let start = text
        .find("<?xml version")
        .ok_or("Missing hdiutil output plist")?;
    let end = text[start..]
        .find("</plist>")
        .ok_or("Incomplete hdiutil output plist")?
        + start
        + 8;
    Value::from_reader_xml(text[start..end].as_bytes()).map_err(|e| e.to_string())
}
pub struct Mount {
    root: PathBuf,
    roots: Vec<PathBuf>,
    device: Option<String>,
    // Only roots created beneath this private directory belong to this guard.
    // hdiutil can return an existing attachment despite a preceding info query.
    _directory: MountDirectory,
    attached: bool,
}
impl Mount {
    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn paths(&self) -> &[PathBuf] {
        &self.roots
    }
    pub fn new(image: &str) -> Result<Self> {
        mac()?;
        let info = Command::new("/usr/bin/hdiutil")
            .args(["imageinfo", image, "-plist"])
            .output()
            .map_err(|e| e.to_string())?;
        if !info.stderr.is_empty() {
            super::processor_output(
                1,
                format!(
                    "hdiutil imageinfo error {} with image {image}.",
                    String::from_utf8_lossy(&info.stderr)
                ),
            );
        }
        let sla = parse_hdiutil_plist(&info.stdout)
            .ok()
            .and_then(|v| {
                v.as_dictionary()
                    .and_then(|d| d.get("Properties"))
                    .and_then(Value::as_dictionary)
                    .and_then(|d| d.get("Software License Agreement"))
                    .and_then(Value::as_boolean)
            })
            .unwrap_or(false);
        let directory = MountDirectory(
            tempfile::Builder::new()
                .prefix("autopkg-mount-")
                .tempdir_in("/private/tmp")
                .map_err(|e| e.to_string())?
                .keep(),
        );
        let mut child = Command::new("/usr/bin/hdiutil")
            .args([
                "attach",
                "-plist",
                "-mountrandom",
                directory.path().to_str().ok_or("Invalid mount directory")?,
                "-nobrowse",
                image,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        if sla {
            if let Some(mut input) = child.stdin.take() {
                let _ = input.write_all(b"Y\n");
            }
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "mounting {image} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let plist = parse_hdiutil_plist(&output.stdout)?;
        let entities = plist
            .as_dictionary()
            .and_then(|d| d.get("system-entities"))
            .and_then(Value::as_array)
            .ok_or("Mounting failed: unexpected output from hdiutil")?;
        let roots: Vec<PathBuf> = entities
            .iter()
            .filter_map(|p| {
                p.as_dictionary()
                    .and_then(|d| d.get("mount-point"))
                    .and_then(Value::as_string)
                    .map(PathBuf::from)
            })
            .collect();
        let root = roots
            .first()
            .ok_or("Mounting failed: no mounted volume")?
            .clone();
        let attached = roots.iter().all(|p| p.starts_with(directory.path()));
        let device = entities
            .iter()
            .filter_map(|p| {
                p.as_dictionary()
                    .and_then(|d| d.get("dev-entry"))
                    .and_then(Value::as_string)
            })
            .find(|p| {
                p.strip_prefix("/dev/disk").is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                })
            })
            .map(str::to_owned);
        super::processor_output(1, format!("Mounted disk image {image}"));
        Ok(Self {
            root,
            roots,
            device,
            _directory: directory,
            attached,
        })
    }
    pub fn resolve(&self, inner: &str) -> Result<String> {
        let inner = inner.replace('\\', "/");
        if inner.starts_with('/') || inner.split('/').any(|c| c == "..") {
            return Err(
                "DMG paths must be relative and may not contain parent-directory references".into(),
            );
        }
        let path = self.root.join(inner);
        let root = io(self.root.canonicalize())?;
        // Validate the nearest existing ancestor and every glob match, including
        // symlinks, before exposing the mounted path to a processor.
        for ancestor in path.ancestors() {
            if ancestor.exists() {
                if !io(ancestor.canonicalize())?.starts_with(&root) {
                    return Err("DMG path resolves outside the mounted image".into());
                }
                break;
            }
        }
        for entry in glob::glob(&path.to_string_lossy()).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !io(entry.canonicalize())?.starts_with(&root) {
                return Err("DMG path resolves outside the mounted image".into());
            }
        }
        Ok(path.to_string_lossy().into_owned())
    }
    pub fn detach(&mut self) -> Result<()> {
        if !self.attached {
            return Ok(());
        }
        let mut output = Command::new("/usr/bin/hdiutil")
            .arg("detach")
            .arg(self.device.as_deref().map(Path::new).unwrap_or(&self.root))
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            output = Command::new("/usr/bin/hdiutil")
                .arg("detach")
                .arg(self.device.as_deref().map(Path::new).unwrap_or(&self.root))
                .arg("-force")
                .output()
                .map_err(|e| e.to_string())?;
        }
        if !output.status.success() {
            return Err(format!(
                "unmounting {} failed: {}",
                self.root.display(),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        self.attached = false;
        Ok(())
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        if self.attached {
            if let Err(error) = self.detach() {
                eprintln!("WARNING: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_directory_cleanup_never_recurses() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("mount-root");
        let volume = root.join("volume");
        std::fs::create_dir_all(&volume).unwrap();
        std::fs::write(volume.join("keep"), b"mounted data").unwrap();
        drop(MountDirectory(root));
        assert_eq!(std::fs::read(volume.join("keep")).unwrap(), b"mounted data");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires hdiutil; creates only an isolated synthetic image"]
    fn reused_attachment_is_not_detached() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("keep"), b"test").unwrap();
        let image = temp.path().join("test.dmg");
        let output = Command::new("/usr/bin/hdiutil")
            .args(["create", "-srcfolder"])
            .arg(&source)
            .args(["-format", "UDZO"])
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut first = Mount::new(image.to_str().unwrap()).unwrap();
        let first_path = first.path().to_owned();
        {
            // hdiutil versions may reuse, create another attachment, or reject
            // a second attach as busy. Every outcome must preserve the first.
            if let Ok(mut second) = Mount::new(image.to_str().unwrap()) {
                if second.path() == first_path {
                    assert!(!second.attached);
                }
                second.detach().unwrap();
            }
        }
        assert!(first_path.join("keep").is_file());
        first.detach().unwrap();
        first.detach().unwrap();
        assert!(!first_path.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Requires hdiutil and diskutil; partitions only a newly created synthetic disk image"]
    fn multi_volume_attachment_detaches_whole_image() {
        fn run(program: &str, args: &[&str]) -> Vec<u8> {
            let output = Command::new(program).args(args).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        }
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("multiple.dmg");
        let image = image.to_str().unwrap();
        run(
            "/usr/bin/hdiutil",
            &["create", "-size", "64m", "-layout", "NONE", image],
        );
        let attachment = parse_hdiutil_plist(&run(
            "/usr/bin/hdiutil",
            &["attach", "-nomount", "-plist", image],
        ))
        .unwrap();
        let device = attachment.as_dictionary().unwrap()["system-entities"]
            .as_array()
            .unwrap()[0]
            .as_dictionary()
            .unwrap()["dev-entry"]
            .as_string()
            .unwrap();
        assert!(device
            .strip_prefix("/dev/disk")
            .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())));
        // `device` comes exclusively from attaching this test's fresh image.
        run(
            "/usr/sbin/diskutil",
            &[
                "partitionDisk",
                device,
                "2",
                "GPT",
                "HFS+",
                "First",
                "50%",
                "HFS+",
                "Second",
                "R",
            ],
        );
        // partitionDisk mounts the new volumes. Release those setup mounts
        // before detaching this validated, freshly attached fixture device.
        run("/usr/sbin/diskutil", &["unmountDisk", "force", device]);
        run("/usr/bin/hdiutil", &["detach", device]);
        let mut mounted = Mount::new(image).unwrap();
        let roots = mounted.paths().to_vec();
        assert_eq!(roots.len(), 2);
        assert_eq!(mounted.path(), roots[0]);
        assert!(roots.iter().all(|p| p.is_dir()));
        let directory = mounted._directory.path().to_owned();
        mounted.detach().unwrap();
        assert!(roots.iter().all(|p| !p.exists()));
        drop(mounted);
        assert!(!directory.exists());
    }
}
