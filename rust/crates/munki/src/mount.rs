//! Munki borrows existing image mounts; only mounts created here are detached.
use plist::Value;
use std::path::{Component, Path, PathBuf};
pub(crate) struct Mount {
    owned: Option<autopkg_platform::dmg::Mount>,
    root: PathBuf,
}
impl Mount {
    pub(crate) fn new(image: &str) -> Result<Self, String> {
        if !cfg!(target_os = "macos") {
            return Err("Disk image operations are only supported on macOS".into());
        }
        let image_path = Path::new(image).canonicalize().map_err(|e| e.to_string())?;
        let output =
            crate::metadata::command("/usr/bin/hdiutil", &["info".as_ref(), "-plist".as_ref()])?;
        let info = autopkg_platform::dmg::parse_hdiutil_plist(&output)?;
        for item in info
            .as_dictionary()
            .and_then(|d| d.get("images"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_dictionary)
        {
            if item
                .get("image-path")
                .and_then(Value::as_string)
                .and_then(|p| Path::new(p).canonicalize().ok())
                .as_ref()
                != Some(&image_path)
            {
                continue;
            }
            if let Some(root) = item
                .get("system-entities")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_dictionary)
                .filter_map(|d| d.get("mount-point").and_then(Value::as_string))
                .find(|p| Path::new(p).is_dir())
            {
                return Ok(Self {
                    owned: None,
                    root: PathBuf::from(root),
                });
            }
        }
        let owned = autopkg_platform::dmg::Mount::new(image)?;
        let root = owned.path().to_owned();
        Ok(Self {
            owned: Some(owned),
            root,
        })
    }
    pub(crate) fn path(&self) -> &Path {
        &self.root
    }
    pub(crate) fn resolve(&self, inner: &str) -> Result<String, String> {
        if let Some(owned) = &self.owned {
            return owned.resolve(inner);
        }
        if Path::new(inner)
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        {
            return Err("DMG paths must be relative without parent-directory references".into());
        }
        let path = self.root.join(inner);
        let root = self.root.canonicalize().map_err(|e| e.to_string())?;
        for ancestor in path.ancestors() {
            if ancestor.exists() {
                if !ancestor
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(&root)
                {
                    return Err("DMG path resolves outside mounted image".into());
                }
                break;
            }
        }
        Ok(path.to_string_lossy().into_owned())
    }
    pub(crate) fn detach(&mut self) -> Result<(), String> {
        if let Some(owned) = &mut self.owned {
            owned.detach()?;
        }
        Ok(())
    }
}
