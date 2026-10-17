//! Russet's preferences, with `--prefs FILE` on top. Every change Russet makes
//! goes to `--prefs FILE` when given, otherwise to Russet's own preferences.
//! AutoPkg's preferences are read only by `russet migrate`, and by
//! `repo-delete` to keep the folders of clones that AutoPkg uses.
use crate::manage::expand;
use autopkg_engine::preferences::{Format, Preferences};
#[cfg(target_os = "macos")]
use autopkg_platform::{AUTOPKG_DOMAIN, RUSSET_DOMAIN};
use plist::Dictionary;
use plist::Value;
use std::{
    cell::OnceCell,
    env, fs,
    path::{Path, PathBuf},
};

/// Where a layer's changes are saved.
enum Target {
    File(PathBuf, Format),
    #[cfg(target_os = "macos")]
    Domain(&'static str),
}

/// One preference source: a file or a native macOS domain.
pub(crate) struct Layer {
    pub(crate) values: Dictionary,
    target: Target,
}

impl Layer {
    /// An existing preference file.
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let file = Preferences::load(path)?;
        Ok(Self {
            values: file.values,
            target: Target::File(file.path, file.format),
        })
    }
    /// A preference file that is created on the first save. A `.json` name
    /// selects JSON; any other name selects a property list.
    pub(crate) fn file(path: &Path) -> Result<Self, String> {
        if path.exists() {
            return Self::load(path);
        }
        let format = if path.extension().is_some_and(|e| e == "json") {
            Format::Json
        } else {
            Format::Plist
        };
        Ok(Self {
            values: Dictionary::new(),
            target: Target::File(path.to_owned(), format),
        })
    }
    /// `config.plist` or `config.json` in `directory`; a new `config.json`
    /// when neither has values.
    #[cfg(any(not(target_os = "macos"), test))]
    fn directory(directory: &Path) -> Result<Self, String> {
        match Preferences::load_directory(directory)? {
            Some(file) => Ok(Self {
                values: file.values,
                target: Target::File(file.path, file.format),
            }),
            None => Self::file(&directory.join("config.json")),
        }
    }
    /// Like AutoPkg, a domain holds the keys set for the current user and for
    /// all users (`/Library/Preferences`). The keys AutoPkg and Russet read by
    /// name are added too, so configuration profiles that set only those keys
    /// still apply.
    #[cfg(target_os = "macos")]
    fn domain(domain: &'static str) -> Result<Self, String> {
        let mut values = Dictionary::new();
        let mut keys = native_keys(domain)?;
        keys.extend(NAMED_KEYS.iter().map(|key| (*key).to_owned()));
        for key in keys {
            if values.contains_key(&key) {
                continue;
            }
            if let Some(value) = autopkg_platform::preference(domain, &key)? {
                values.insert(key, value);
            }
        }
        Ok(Self {
            values,
            target: Target::Domain(domain),
        })
    }
    /// Write `keys` to a domain, or the whole layer to a file.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub(crate) fn save(&self, keys: &[&str]) -> Result<(), String> {
        match &self.target {
            Target::File(path, format) => {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                Preferences {
                    values: self.values.clone(),
                    path: path.clone(),
                    format: *format,
                }
                .save()
            }
            #[cfg(target_os = "macos")]
            Target::Domain(domain) => {
                for key in keys {
                    native_set(
                        domain,
                        key,
                        self.values.get(key).ok_or("Missing preference value")?,
                    )?;
                }
                Ok(())
            }
        }
    }
    /// Whether the values come from a macOS preference domain.
    fn is_domain(&self) -> bool {
        match &self.target {
            Target::File(..) => false,
            #[cfg(target_os = "macos")]
            Target::Domain(_) => true,
        }
    }
    /// Where this layer is saved, for messages.
    pub(crate) fn location(&self) -> String {
        match &self.target {
            Target::File(path, _) => path.display().to_string(),
            #[cfg(target_os = "macos")]
            Target::Domain(domain) => format!("the {domain} preference domain"),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn config_root() -> Result<PathBuf, String> {
    if cfg!(windows) {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "LOCALAPPDATA is unavailable".into())
    } else {
        Ok(env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or(crate::manage::home()?.join(".config")))
    }
}

/// Preferences that AutoPkg or Russet read by name rather than from the list
/// of keys in a domain.
#[cfg(target_os = "macos")]
const NAMED_KEYS: [&str; 16] = [
    "CACHE_DIR",
    "CURL_PATH",
    "DISABLE_RECIPE_MAP",
    "FAIL_RECIPES_WITHOUT_TRUST_INFO",
    "GITHUB_TOKEN",
    "GIT_PATH",
    "MUNKI_REPO",
    "MUNKI_REPO_PLUGIN",
    "RECIPE_MAP_PATH",
    "RECIPE_OVERRIDE_DIRS",
    "RECIPE_OVERRIDE_FORMAT",
    "RECIPE_REPOS",
    "RECIPE_REPO_DIR",
    "RECIPE_SEARCH_DIRS",
    "RussetJobs",
    "UseRussetDownloader",
];

/// AutoPkg's `config.plist` or `config.json`, read the way AutoPkg reads
/// them: a file that's missing, empty, or unreadable is skipped.
#[cfg(any(not(target_os = "macos"), test))]
fn autopkg_directory(directory: &Path) -> Layer {
    for name in ["config.plist", "config.json"] {
        if let Ok(file) = Preferences::load(&directory.join(name)) {
            if !file.values.is_empty() {
                return Layer {
                    values: file.values,
                    target: Target::File(file.path, file.format),
                };
            }
        }
    }
    Layer {
        values: Dictionary::new(),
        target: Target::File(directory.join("config.plist"), Format::Plist),
    }
}

/// AutoPkg's preferences, which Russet only reads.
/// `RUSSET_AUTOPKG_PREFERENCES_FILE` replaces them, so tests never read the
/// user's settings.
pub(crate) fn autopkg_layer() -> Result<Layer, String> {
    if let Some(path) = env::var_os("RUSSET_AUTOPKG_PREFERENCES_FILE").filter(|p| !p.is_empty()) {
        return Layer::load(Path::new(&path));
    }
    #[cfg(target_os = "macos")]
    return Layer::domain(AUTOPKG_DOMAIN);
    #[cfg(not(target_os = "macos"))]
    Ok(autopkg_directory(&config_root()?.join("Autopkg")))
}

/// Russet's own preferences. `RUSSET_PREFERENCES_FILE` replaces them, so tests
/// never read or change the user's settings.
pub(crate) fn russet_layer() -> Result<Layer, String> {
    if let Some(path) = env::var_os("RUSSET_PREFERENCES_FILE").filter(|p| !p.is_empty()) {
        return Layer::file(Path::new(&path));
    }
    #[cfg(target_os = "macos")]
    return Layer::domain(RUSSET_DOMAIN);
    #[cfg(not(target_os = "macos"))]
    Layer::directory(&config_root()?.join("Russet"))
}

/// A domain value after AutoPkg's `__deepconvert_objc`, which truncates
/// numbers to integers at the top level and inside arrays.
fn autopkg_numbers(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(autopkg_numbers).collect()),
        other => crate::pprint::autopkg_number(other),
    }
}

/// The merged preferences and the layer that receives changes.
pub(crate) struct Store {
    pub(crate) values: Dictionary,
    /// Keys whose values come from a macOS domain. AutoPkg converts these
    /// with PyObjC, so `info` shows them the same way.
    pub(crate) native: Vec<String>,
    russet: Layer,
    explicit: Option<Layer>,
    /// The recipe repositories in AutoPkg's preferences, read on first use.
    autopkg_repos: OnceCell<Dictionary>,
}

impl Store {
    pub(crate) fn load(explicit: Option<&str>) -> Result<Self, String> {
        let explicit = explicit
            .map(|path| Layer::load(&expand(path)?))
            .transpose()?;
        Ok(Self::new(russet_layer()?, explicit))
    }
    pub(crate) fn new(russet: Layer, explicit: Option<Layer>) -> Self {
        let mut values = russet.values.clone();
        let mut native = Vec::new();
        if russet.is_domain() {
            for (key, value) in values.iter_mut() {
                *value = autopkg_numbers(value);
                native.push(key.clone());
            }
        }
        if let Some(explicit) = &explicit {
            native.retain(|key| !explicit.values.contains_key(key));
            values.extend(explicit.values.clone());
        }
        Self {
            values,
            native,
            russet,
            explicit,
            autopkg_repos: OnceCell::new(),
        }
    }
    /// Whether AutoPkg's preferences list a recipe repository while Russet
    /// saves to its own preferences, so the clone belongs to AutoPkg too.
    /// AutoPkg's preferences that can't be read count as listing nothing.
    pub(crate) fn shared_with_autopkg(&self, path: &str) -> bool {
        self.explicit.is_none()
            && self
                .autopkg_repos
                .get_or_init(|| {
                    autopkg_layer()
                        .ok()
                        .and_then(|layer| {
                            layer
                                .values
                                .get("RECIPE_REPOS")
                                .and_then(Value::as_dictionary)
                                .cloned()
                        })
                        .unwrap_or_default()
                })
                .contains_key(path)
    }
    #[cfg(test)]
    pub(crate) fn set_autopkg_repos(&mut self, repos: Dictionary) {
        self.autopkg_repos = OnceCell::from(repos);
    }
    /// Save the merged value of each key to `--prefs FILE` or Russet's config.
    pub(crate) fn save(&mut self, keys: &[&str]) -> Result<(), String> {
        let layer = self.explicit.as_mut().unwrap_or(&mut self.russet);
        for key in keys {
            let value = self.values.get(key).ok_or("Missing preference value")?;
            layer.values.insert((*key).into(), value.clone());
        }
        layer.save(keys)
    }
}

#[cfg(target_os = "macos")]
fn native_keys(domain: &str) -> Result<Vec<String>, String> {
    use core_foundation_sys::{
        array::{CFArrayGetCount, CFArrayGetValueAtIndex},
        base::CFRelease,
        preferences::{
            kCFPreferencesAnyHost, kCFPreferencesAnyUser, kCFPreferencesCurrentHost,
            kCFPreferencesCurrentUser, CFPreferencesCopyKeyList,
        },
        string::{
            kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString,
            CFStringGetLength, CFStringGetMaximumSizeForEncoding,
        },
    };
    let domain = std::ffi::CString::new(domain).map_err(|e| e.to_string())?;
    unsafe {
        let domain =
            CFStringCreateWithCString(std::ptr::null(), domain.as_ptr(), kCFStringEncodingUTF8);
        if domain.is_null() {
            return Err("Cannot allocate preference domain".into());
        }
        let mut result = Vec::new();
        // The same two scopes AutoPkg lists: `/Library/Preferences`, then the
        // current user's preferences.
        for (user, host) in [
            (kCFPreferencesAnyUser, kCFPreferencesCurrentHost),
            (kCFPreferencesCurrentUser, kCFPreferencesAnyHost),
        ] {
            let array = CFPreferencesCopyKeyList(domain, user, host);
            if array.is_null() {
                continue;
            }
            for index in 0..CFArrayGetCount(array) {
                let key = CFArrayGetValueAtIndex(array, index).cast();
                let size = CFStringGetMaximumSizeForEncoding(
                    CFStringGetLength(key),
                    kCFStringEncodingUTF8,
                ) + 1;
                let mut bytes = vec![0u8; size as usize];
                if CFStringGetCString(key, bytes.as_mut_ptr().cast(), size, kCFStringEncodingUTF8)
                    != 0
                {
                    let key = std::ffi::CStr::from_ptr(bytes.as_ptr().cast())
                        .to_string_lossy()
                        .into_owned();
                    if !result.contains(&key) {
                        result.push(key);
                    }
                }
            }
            CFRelease(array.cast());
        }
        CFRelease(domain.cast());
        Ok(result)
    }
}

#[cfg(target_os = "macos")]
fn native_set(domain: &str, key: &str, value: &Value) -> Result<(), String> {
    use core_foundation_sys::{
        base::{CFRelease, CFTypeRef},
        data::CFDataCreate,
        preferences::{CFPreferencesAppSynchronize, CFPreferencesSetAppValue},
        propertylist::CFPropertyListCreateWithData,
        string::{kCFStringEncodingUTF8, CFStringCreateWithCString},
    };
    use std::{ffi::CString, ptr};
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }
    // Writing AutoPkg's domain would change a side-by-side AutoPkg install.
    if domain == AUTOPKG_DOMAIN {
        return Err(format!("Russet never changes the {AUTOPKG_DOMAIN} domain"));
    }
    let mut bytes = Vec::new();
    value.to_writer_xml(&mut bytes).map_err(|e| e.to_string())?;
    let domain = CString::new(domain).map_err(|e| e.to_string())?;
    let key = CString::new(key).map_err(|e| e.to_string())?;
    unsafe {
        let domain = Owned(
            CFStringCreateWithCString(ptr::null(), domain.as_ptr(), kCFStringEncodingUTF8).cast(),
        );
        let key = Owned(
            CFStringCreateWithCString(ptr::null(), key.as_ptr(), kCFStringEncodingUTF8).cast(),
        );
        let data = Owned(CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize).cast());
        if domain.0.is_null() || key.0.is_null() || data.0.is_null() {
            return Err("Cannot allocate native preference".into());
        }
        let mut error = ptr::null_mut();
        let value = Owned(CFPropertyListCreateWithData(
            ptr::null(),
            data.0.cast(),
            0,
            ptr::null_mut(),
            &mut error,
        ));
        if !error.is_null() {
            CFRelease(error.cast());
        }
        if value.0.is_null() {
            return Err("Cannot decode native preference".into());
        }
        CFPreferencesSetAppValue(key.0.cast(), value.0, domain.0.cast());
        if CFPreferencesAppSynchronize(domain.0.cast()) == 0 {
            return Err("Could not synchronize native preferences".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, values: &[(&str, &str)]) {
        Value::Dictionary(Dictionary::from_iter(
            values.iter().map(|(k, v)| (*k, Value::String((*v).into()))),
        ))
        .to_file_xml(path)
        .unwrap();
    }

    #[test]
    fn changes_go_to_russet() {
        let temp = tempfile::tempdir().unwrap();
        let russet_path = temp.path().join("new/russet.plist");
        let mut store = Store::new(Layer::file(&russet_path).unwrap(), None);
        assert!(store.values.is_empty());
        store.values.insert("A".into(), "changed".into());
        store.save(&["A"]).unwrap();
        let saved = Preferences::load(&russet_path).unwrap().values;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved["A"].as_string(), Some("changed"));
        let reloaded = Store::new(Layer::load(&russet_path).unwrap(), None);
        assert_eq!(reloaded.values["A"].as_string(), Some("changed"));
    }

    #[test]
    fn explicit_file_wins_and_receives_only_its_values_and_changes() {
        let temp = tempfile::tempdir().unwrap();
        let russet_path = temp.path().join("russet.plist");
        write(&russet_path, &[("RUSSET", "secret"), ("OWN", "russet")]);
        let russet_before = fs::read(&russet_path).unwrap();
        let explicit_path = temp.path().join("explicit.plist");
        write(&explicit_path, &[("OWN", "value")]);
        let mut store = Store::new(
            Layer::load(&russet_path).unwrap(),
            Some(Layer::load(&explicit_path).unwrap()),
        );
        assert_eq!(store.values["RUSSET"].as_string(), Some("secret"));
        assert_eq!(store.values["OWN"].as_string(), Some("value"));
        store.values.insert("NEW".into(), "added".into());
        store.save(&["NEW"]).unwrap();
        let saved = Preferences::load(&explicit_path).unwrap().values;
        assert_eq!(
            saved.keys().map(String::as_str).collect::<Vec<_>>(),
            ["OWN", "NEW"]
        );
        assert_eq!(fs::read(&russet_path).unwrap(), russet_before);
        assert!(!store.shared_with_autopkg("anything"));
    }

    #[test]
    fn missing_config_directory_creates_json() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("Russet");
        let mut layer = Layer::directory(&directory).unwrap();
        assert!(layer.values.is_empty());
        layer.values.insert("KEY".into(), "value".into());
        layer.save(&["KEY"]).unwrap();
        let saved = Preferences::load(&directory.join("config.json")).unwrap();
        assert_eq!(saved.format, Format::Json);
        assert_eq!(saved.values["KEY"].as_string(), Some("value"));
    }

    #[test]
    fn domain_numbers_are_truncated_like_autopkg() {
        let value = Value::Array(vec![
            Value::Real(2.75),
            Value::Real(-2.5),
            Value::Array(vec![Value::Real(4.75)]),
            Value::Dictionary(Dictionary::from_iter([("f", Value::Real(3.25))])),
        ]);
        assert_eq!(
            autopkg_numbers(&value),
            Value::Array(vec![
                Value::from(2),
                Value::from(-2),
                Value::Array(vec![Value::from(4)]),
                Value::Dictionary(Dictionary::from_iter([("f", Value::Real(3.25))])),
            ])
        );
        assert_eq!(autopkg_numbers(&Value::Real(1.9)), Value::from(1));
    }

    #[test]
    fn autopkg_directory_skips_unreadable_and_empty_files_like_autopkg() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path();
        assert!(autopkg_directory(directory).values.is_empty());
        fs::write(directory.join("config.json"), r#"{"KEY": "json"}"#).unwrap();
        for plist in ["not a plist", "<plist><dict/></plist>"] {
            fs::write(directory.join("config.plist"), plist).unwrap();
            assert_eq!(
                autopkg_directory(directory).values["KEY"].as_string(),
                Some("json")
            );
        }
        write(&directory.join("config.plist"), &[("KEY", "plist")]);
        assert_eq!(
            autopkg_directory(directory).values["KEY"].as_string(),
            Some("plist")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn never_writes_autopkg_domain() {
        assert!(native_set(AUTOPKG_DOMAIN, "KEY", &Value::from("value")).is_err());
    }
}
