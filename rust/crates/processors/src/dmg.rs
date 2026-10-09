//! Scoped disk-image mounts are detached on success and on processor errors.
use super::Result;
#[cfg(all(test, target_os = "macos"))]
use autopkg_platform::dmg::parse_hdiutil_plist as first_plist;
pub(crate) use autopkg_platform::dmg::Mount;
use plist::{Dictionary, Value};
pub(crate) fn split(path: &str) -> Option<(&str, &str)> {
    for extension in [".dmg", ".iso", ".DMG", ".ISO"] {
        for separator in ['/', '\\'] {
            if let Some(index) = path.find(&format!("{extension}{separator}")) {
                let end = index + extension.len();
                return Some((&path[..end], &path[end + 1..]));
            }
        }
    }
    None
}
pub(crate) fn input_key(name: &str) -> Option<&'static str> {
    match name {
        "CodeSignatureVerifier" => Some("input_path"),
        "Copier" => Some("source_path"),
        "FileFinder" => Some("pattern"),
        "PlistReader" => Some("info_path"),
        "Versioner" => Some("input_plist_path"),
        "FlatPkgUnpacker" => Some("flat_pkg_path"),
        "PkgCopier" => Some("source_pkg"),
        "PkgExtractor" | "Installer" => Some("pkg_path"),
        _ => None,
    }
}
thread_local! { static RESOLVING_INPUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
struct ResolvedInput(bool);
impl Drop for ResolvedInput {
    fn drop(&mut self) {
        RESOLVING_INPUT.with(|value| value.set(self.0));
    }
}
pub(crate) fn run_mounted(
    name: &str,
    env: &mut Dictionary,
    preferences: Option<&Dictionary>,
    standalone: bool,
) -> Option<Result<()>> {
    let key = input_key(name)?;
    if name == "Installer" && !cfg!(target_os = "macos") {
        // Installing needs macOS; fail before reading the image.
        return None;
    }
    let original = env.get(key)?.as_string()?.to_string();
    let parsed = split(&original);
    if name == "Copier" && !RESOLVING_INPUT.with(std::cell::Cell::get) {
        let (image, separator, inner) = if let Some((image, inner)) = parsed {
            (image, &original[image.len() - 4..image.len() + 1], inner)
        } else {
            (original.as_str(), "", "")
        };
        autopkg_platform::processor_output(
            2,
            format!(
                "Parsed dmg results: dmg_path: {image}, dmg: {separator}, dmg_source_path: {inner}"
            ),
        );
    }
    let (image, inner) = parsed?;
    Some((|| {
        if name == "FileFinder" {
            let method = env
                .get("find_method")
                .and_then(Value::as_string)
                .unwrap_or("glob");
            if method != "glob" {
                return Err(format!("Unsupported find_method: {method}"));
            }
        }
        let mut mount = Mount::new(image)?;
        let resolved = mount.resolve(inner)?;
        if name == "FileFinder" {
            let result = (|| {
                let mut paths = super::matches(&resolved)?;
                paths.sort();
                let path = paths.last().ok_or("No matching filename found")?;
                env.insert(
                    "found_filename".into(),
                    path.to_string_lossy().into_owned().into(),
                );
                autopkg_platform::processor_output(
                    1,
                    format!(
                        "Found file match: '{}' from globbed '{resolved}'",
                        path.display()
                    ),
                );
                if let Ok(relative) = path.strip_prefix(mount.path()) {
                    let relative = relative.to_string_lossy().into_owned();
                    env.insert("dmg_found_filename".into(), relative.clone().into());
                    autopkg_platform::processor_output(
                        1,
                        format!("DMG-relative file match: '{relative}'"),
                    );
                }
                let basename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                env.insert("found_basename".into(), basename.clone().into());
                autopkg_platform::processor_output(1, format!("Basename match: '{basename}'"));
                Ok(())
            })();
            return mount.detach().and(result);
        }
        // FlatPkgUnpacker expands globs only for paths inside a disk image.
        // Keep the mount alive through extraction and leave ordinary filesystem
        // paths literal, matching the Python processor.
        let resolved = if name == "FlatPkgUnpacker" {
            flat_package_path(&resolved, &original)?
        } else {
            resolved
        };
        env.insert(key.into(), resolved.into());
        let _resolved = ResolvedInput(RESOLVING_INPUT.with(|value| value.replace(true)));
        let result = super::execute_builtin(name, env, preferences, standalone);
        env.insert(key.into(), original.clone().into());
        let cleanup = mount.detach();
        cleanup.and(result)
    })())
}
fn flat_package_path(pattern: &str, original: &str) -> Result<String> {
    let paths = super::python_glob::paths_with_recursion(pattern, false)?;
    match paths.as_slice() {
        [] => Err(format!(
            "No valid path found as given by 'flat_pkg_path': {original}"
        )),
        [path] => Ok(path.to_string_lossy().into_owned()),
        _ => Err(format!(
            "Multiple source paths found in globbed path in 'flat_pkg_path'. There must be only one. Found: {}",
            paths.iter().map(|path| path.to_string_lossy()).collect::<Vec<_>>().join(", ")
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use crate::tests::{env, Temp};
    #[allow(unused_imports)]
    use std::process::Command;
    #[test]
    fn splitting_handles_paths_and_case() {
        assert_eq!(split("/tmp/a.DMG/thing"), Some(("/tmp/a.DMG", "thing")));
        assert_eq!(split("plain.pkg"), None);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn create_copy_and_detach_on_failure() {
        let t = Temp::new();
        let root = t.0.join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("hello"), "disk fixture").unwrap();
        std::fs::create_dir_all(root.join("Fixture.app/Contents")).unwrap();
        let mut info = Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "org.autopkg.fixture".into());
        info.insert("CFBundleShortVersionString".into(), "1.2.3".into());
        Value::Dictionary(info)
            .to_file_xml(root.join("Fixture.app/Contents/Info.plist"))
            .unwrap();
        let image = t.path("fixture.dmg");
        let mut e = env(&[("dmg_root", root.to_str().unwrap()), ("dmg_path", &image)]);
        super::super::execute("DmgCreator", &mut e).unwrap();
        super::super::execute("AppDmgVersioner", &mut e).unwrap();
        assert_eq!(e["bundleid"].as_string(), Some("org.autopkg.fixture"));
        assert_eq!(e["version"].as_string(), Some("1.2.3"));
        e.insert("source_path".into(), format!("{image}/hello").into());
        e.insert("destination_path".into(), t.path("copied").into());
        super::super::execute("Copier", &mut e).unwrap();
        assert_eq!(
            std::fs::read_to_string(t.path("copied")).unwrap(),
            "disk fixture"
        );
        e.insert("source_path".into(), format!("{image}/missing").into());
        assert!(super::super::execute("Copier", &mut e).is_err());
        let info = Command::new("/usr/bin/hdiutil")
            .args(["info", "-plist"])
            .output()
            .unwrap();
        let plist = first_plist(&info.stdout).unwrap();
        let images = plist.as_dictionary().unwrap()["images"].as_array().unwrap();
        assert!(!images.iter().any(|v| v
            .as_dictionary()
            .and_then(|d| d.get("image-path"))
            .and_then(Value::as_string)
            == Some(image.as_str())));
    }
}
