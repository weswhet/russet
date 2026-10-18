//! Scoped native disk-image mounts shared by processors and metadata inspection.
use plist::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
type Result<T> = std::result::Result<T, String>;

/// Images in use, keyed by canonical path. When recipes run in parallel,
/// one recipe at a time uses an image: otherwise `hdiutil attach` can hand a
/// recipe another recipe's attachment, which the other recipe then detaches
/// while it's still in use.
static IMAGES: crate::serial::KeyedLock<PathBuf> = crate::serial::KeyedLock::new();

/// Holds `image` until the guard drops, waiting while another thread uses
/// it. A thread can hold the same image more than once.
pub fn lock_image(image: &str) -> crate::serial::KeyedGuard<PathBuf> {
    let path = Path::new(image);
    IMAGES.lock(path.canonicalize().unwrap_or_else(|_| path.to_owned()))
}
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
fn unsupported() -> String {
    "Disk image operations are only supported on macOS and Linux".into()
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
    _directory: Option<MountDirectory>,
    /// The extracted image when Russet reads it natively instead of
    /// attaching it.
    #[cfg(unix)]
    _extraction: Option<std::sync::Arc<native::Extraction>>,
    attached: bool,
    /// Declared last so it's released after the image is detached.
    _image: Option<crate::serial::KeyedGuard<PathBuf>>,
}
impl Mount {
    pub fn path(&self) -> &Path {
        &self.root
    }
    pub fn paths(&self) -> &[PathBuf] {
        &self.roots
    }
    /// Makes an image's volumes available: attached with `hdiutil` on macOS,
    /// or extracted by Russet's native reader on Linux and when
    /// `RUSSET_NATIVE` names `hdiutil`.
    pub fn new(image: &str) -> Result<Self> {
        use crate::backend::{select, Backend, Tool};
        let lock = lock_image(image);
        let mut mount = match select(Tool::Hdiutil) {
            Backend::Apple => Self::attach(image),
            #[cfg(unix)]
            Backend::Native => {
                let extraction = native::open(image)?;
                super::processor_output(1, format!("Mounted disk image {image}"));
                Ok(Self {
                    root: extraction.volumes[0].clone(),
                    roots: extraction.volumes.clone(),
                    device: None,
                    _directory: None,
                    _extraction: Some(extraction),
                    attached: false,
                    _image: None,
                })
            }
            _ => Err(unsupported()),
        }?;
        mount._image = Some(lock);
        Ok(mount)
    }
    fn attach(image: &str) -> Result<Self> {
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
            _directory: Some(directory),
            #[cfg(unix)]
            _extraction: None,
            attached,
            _image: None,
        })
    }
    pub fn resolve(&self, inner: &str) -> Result<String> {
        let inner = inner.replace('\\', "/");
        if inner.starts_with('/') || inner.split('/').any(|c| c == "..") {
            return Err(
                "DMG paths must be relative and may not contain parent-directory references".into(),
            );
        }
        let path = self.root.join(&inner);
        // An extracted image sits on the host filesystem, which on Linux is
        // case-sensitive, but a mounted HFS+ or APFS volume usually isn't:
        // Reaper's recipe asks for `Reaper.app` in an image that holds
        // `REAPER.app`. Patterns are left to the caller's glob, which falls
        // back to matching regardless of case.
        #[cfg(unix)]
        let path = match self._extraction {
            Some(_) if !inner.contains(['*', '?', '[']) => {
                crate::case_fold::resolve(&path).unwrap_or(path)
            }
            _ => path,
        };
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
        self._image = None;
        Ok(())
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        if self.attached {
            if let Err(error) = self.detach() {
                crate::text_eprintln!("WARNING: {error}");
            }
        }
    }
}

thread_local! {
    /// The scope that owns native extractions this thread makes; 0 is none.
    static SCOPE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Clears cached native extractions when dropped. The engine holds one for
/// each recipe run, so an image opened by several steps of a recipe is
/// extracted once, and the scratch space is released when the recipe ends.
/// Dropping a scope releases only the extractions made under it, so recipes
/// running on other threads keep theirs.
pub struct RecipeScope {
    id: u64,
    previous: u64,
    /// Releases every extraction, not just this scope's.
    all: bool,
    _thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl RecipeScope {
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            id,
            previous: SCOPE.with(|scope| scope.replace(id)),
            all: false,
            _thread_bound: std::marker::PhantomData,
        }
    }

    /// A scope for the whole process, which releases every extraction when
    /// dropped.
    pub fn process() -> Self {
        Self {
            all: true,
            ..Self::new()
        }
    }
}

impl Default for RecipeScope {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RecipeScope {
    fn drop(&mut self) {
        SCOPE.with(|scope| scope.set(self.previous));
        let released = (!self.all).then_some(self.id);
        #[cfg(unix)]
        native::clear_cache(released);
        #[cfg(not(unix))]
        let _ = released;
    }
}

#[cfg(unix)]
mod native {
    use super::Result;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    /// Where extracted images go: `RUSSET_SCRATCH_DIR`, or the system
    /// temporary folder.
    pub(super) const SCRATCH_VARIABLE: &str = "RUSSET_SCRATCH_DIR";

    /// An image extracted into a private scratch folder, removed on drop.
    pub(crate) struct Extraction {
        directory: PathBuf,
        pub(super) volumes: Vec<PathBuf>,
    }

    impl Drop for Extraction {
        fn drop(&mut self) {
            remove_tree(&self.directory);
        }
    }

    /// Removes a tree whose folders may be read-only, without following
    /// symlinks.
    fn remove_tree(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(entries) = std::fs::read_dir(path) {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    remove_tree(&entry.path());
                }
            }
        }
        let _ = std::fs::remove_dir_all(path);
    }

    type Key = (PathBuf, u64, Option<SystemTime>);
    /// Extractions by the scope that uses them. Scopes can share an
    /// extraction, which is removed when the last one drops it.
    static CACHE: Mutex<Vec<(u64, Key, Arc<Extraction>)>> = Mutex::new(Vec::new());

    /// Releases one scope's extractions, or every extraction.
    pub(super) fn clear_cache(scope: Option<u64>) {
        let drained: Vec<_> = {
            let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            let (drained, kept) = std::mem::take(&mut *cache)
                .into_iter()
                .partition(|(owner, _, _)| scope.is_none_or(|scope| scope == *owner));
            *cache = kept;
            drained
        };
        drop(drained);
    }

    pub(super) fn open(image: &str) -> Result<Arc<Extraction>> {
        let path = Path::new(image)
            .canonicalize()
            .map_err(|e| format!("mounting {image} failed: {e}"))?;
        let metadata =
            std::fs::metadata(&path).map_err(|e| format!("mounting {image} failed: {e}"))?;
        let key = (path.clone(), metadata.len(), metadata.modified().ok());
        let scope = super::SCOPE.with(std::cell::Cell::get);
        {
            let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, _, extraction)) = cache.iter().find(|(_, k, _)| *k == key) {
                let extraction = extraction.clone();
                if !cache
                    .iter()
                    .any(|(owner, k, _)| *owner == scope && *k == key)
                {
                    cache.push((scope, key, extraction.clone()));
                }
                return Ok(extraction);
            }
        }
        // The caller holds this image's lock, so no other thread extracts it
        // meanwhile, and other images extract in parallel.
        let base = std::env::var_os(SCRATCH_VARIABLE)
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let directory = tempfile::Builder::new()
            .prefix("russet-image-")
            .tempdir_in(&base)
            .map_err(|e| e.to_string())?
            .keep();
        let mut extraction = Extraction {
            directory,
            volumes: Vec::new(),
        };
        let result =
            russet_hdiutil::extract(&path, &extraction.directory, russet_fs::Limits::default())
                .map_err(|e| format!("mounting {image} failed: {e}"))?;
        for skipped in &result.skipped_xattrs {
            crate::processor_output(
                2,
                format!(
                    "Couldn't keep extended attribute {} on {}: {}",
                    skipped.name,
                    skipped.path.display(),
                    skipped.reason
                ),
            );
        }
        extraction.volumes = result.volumes;
        let extraction = Arc::new(extraction);
        CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((scope, key, extraction.clone()));
        Ok(extraction)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn extraction(root: &Path, name: &str) -> Arc<Extraction> {
            let directory = root.join(name);
            std::fs::create_dir(&directory).unwrap();
            Arc::new(Extraction {
                directory,
                volumes: vec![],
            })
        }

        #[test]
        fn a_scope_releases_only_its_own_extractions() {
            let temp = tempfile::tempdir().unwrap();
            let key = |name: &str| (temp.path().join(name), 0, None);
            let first = super::super::RecipeScope::new();
            let shared = extraction(temp.path(), "shared");
            let own = extraction(temp.path(), "own");
            let (first_id, second_id) = (first.id, first.id + 1_000_000);
            CACHE.lock().unwrap().extend([
                (first_id, key("shared"), shared.clone()),
                (first_id, key("own"), own),
                (second_id, key("shared"), shared.clone()),
            ]);
            drop(shared);
            drop(first);
            // The other scope still uses the shared image.
            assert!(temp.path().join("shared").is_dir());
            assert!(!temp.path().join("own").exists());
            clear_cache(Some(second_id));
            assert!(!temp.path().join("shared").exists());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn an_image_is_locked_by_its_canonical_path() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("image.dmg");
        std::fs::write(&image, b"").unwrap();
        let alias = temp.path().join("alias.dmg");
        std::os::unix::fs::symlink(&image, &alias).unwrap();
        let released = AtomicBool::new(false);
        let held = lock_image(image.to_str().unwrap());
        // The same thread can take it again.
        drop(lock_image(alias.to_str().unwrap()));
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                let _guard = lock_image(alias.to_str().unwrap());
                released.load(Ordering::SeqCst)
            });
            std::thread::sleep(std::time::Duration::from_millis(50));
            released.store(true, Ordering::SeqCst);
            drop(held);
            assert!(waiter.join().unwrap(), "took an image another thread held");
        });
    }

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
        let directory = mounted._directory.as_ref().unwrap().path().to_owned();
        mounted.detach().unwrap();
        assert!(roots.iter().all(|p| !p.exists()));
        drop(mounted);
        assert!(!directory.exists());
    }
}
