//! In-memory FileRepo catalog indexes and duplicate matching.
//! Semantics follow AutoPkg's AutoPkgLib.make_catalog_db and MunkiImporter.
use plist::{Dictionary, Value};
use std::collections::{BTreeMap, BTreeSet};

type Versions = BTreeMap<String, BTreeMap<String, Vec<usize>>>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathMatch {
    pub path: String,
    pub index: usize,
}
#[derive(Clone, Debug, Default)]
pub struct CatalogIndex {
    pub items: Vec<Value>,
    pub hashes: BTreeMap<String, Vec<usize>>,
    pub receipts: Versions,
    pub applications: Versions,
    pub installer_items: Versions,
    pub checksums: BTreeMap<String, Vec<PathMatch>>,
    pub files: BTreeMap<String, Vec<PathMatch>>,
}
fn text<'a>(d: &'a Dictionary, key: &str) -> Option<&'a str> {
    d.get(key).and_then(Value::as_string)
}
fn entries<'a>(d: &'a Dictionary, key: &str) -> impl Iterator<Item = &'a Dictionary> {
    d.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_dictionary)
}
fn insert(table: &mut Versions, key: &str, version: &str, index: usize) {
    table
        .entry(key.into())
        .or_default()
        .entry(version.into())
        .or_default()
        .push(index);
}
fn version(item: &Dictionary) -> Option<&str> {
    text(
        item,
        text(item, "version_comparison_key").unwrap_or("CFBundleShortVersionString"),
    )
}
fn intersection(current: &mut BTreeSet<usize>, values: &[usize]) {
    // Preserve the reference's reset when an earlier intersection is empty.
    if current.is_empty() {
        current.extend(values.iter().copied());
    } else {
        let values: BTreeSet<usize> = values.iter().copied().collect();
        current.retain(|i| values.contains(i));
    }
}

impl CatalogIndex {
    /// Reference-shaped diagnostic representation of the in-memory database.
    pub fn to_plist(&self) -> Value {
        fn indexes(values: &[usize]) -> Value {
            Value::Array(
                values
                    .iter()
                    .map(|v| Value::Integer((*v as u64).into()))
                    .collect(),
            )
        }
        fn versions(table: &Versions) -> Value {
            Value::Dictionary(
                table
                    .iter()
                    .map(|(key, versions)| {
                        (
                            key.clone(),
                            Value::Dictionary(
                                versions
                                    .iter()
                                    .map(|(version, values)| (version.clone(), indexes(values)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            )
        }
        fn paths(table: &BTreeMap<String, Vec<PathMatch>>) -> Value {
            Value::Dictionary(
                table
                    .iter()
                    .map(|(key, values)| {
                        (
                            key.clone(),
                            Value::Array(
                                values
                                    .iter()
                                    .map(|v| {
                                        Value::Dictionary(Dictionary::from_iter([
                                            ("path", Value::String(v.path.clone())),
                                            ("index", Value::Integer((v.index as u64).into())),
                                        ]))
                                    })
                                    .collect(),
                            ),
                        )
                    })
                    .collect(),
            )
        }
        Value::Dictionary(Dictionary::from_iter([
            ("items", Value::Array(self.items.clone())),
            (
                "hashes",
                Value::Dictionary(
                    self.hashes
                        .iter()
                        .map(|(key, values)| (key.clone(), indexes(values)))
                        .collect(),
                ),
            ),
            ("receipts", versions(&self.receipts)),
            ("applications", versions(&self.applications)),
            ("installer_items", versions(&self.installer_items)),
            ("checksums", paths(&self.checksums)),
            ("files", paths(&self.files)),
        ]))
    }
    pub fn new(items: Vec<Value>) -> Result<Self, String> {
        let mut result = Self {
            items,
            ..Self::default()
        };
        for (index, item) in result.items.iter().enumerate() {
            let item = item
                .as_dictionary()
                .ok_or("Munki catalog entries must be dictionaries")?;
            let (Some(name), Some(vers)) = (text(item, "name"), text(item, "version")) else {
                continue;
            };
            if name == "NO NAME" || vers == "NO VERSION" {
                continue;
            }
            if let Some(hash) = text(item, "installer_item_hash") {
                result.hashes.entry(hash.into()).or_default().push(index);
            }
            if let Some(location) = text(item, "installer_item_location") {
                insert(
                    &mut result.installer_items,
                    location.rsplit('/').next().unwrap_or(location),
                    vers,
                    index,
                );
            }
            for receipt in entries(item, "receipts") {
                if let (Some(id), Some(v)) = (text(receipt, "packageid"), text(receipt, "version"))
                {
                    insert(&mut result.receipts, id, v, index);
                }
            }
            for install in entries(item, "installs") {
                let Some(path) = text(install, "path") else {
                    continue;
                };
                match text(install, "type") {
                    Some("application" | "bundle") => {
                        if let Some(v) = version(install) {
                            insert(&mut result.applications, path, v, index);
                        }
                    }
                    Some("file") => {
                        let entry = PathMatch {
                            path: path.into(),
                            index,
                        };
                        if let Some(hash) = text(install, "md5checksum") {
                            result.checksums.entry(hash.into()).or_default().push(entry);
                        } else {
                            result.files.entry(path.into()).or_default().push(entry);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(result)
    }
    fn selected(&self, indexes: impl IntoIterator<Item = usize>) -> Vec<Dictionary> {
        indexes
            .into_iter()
            .filter_map(|i| self.items.get(i).and_then(Value::as_dictionary).cloned())
            .collect()
    }
    /// Match in reference order: installer hash, applications, receipts, file
    /// checksums, then unversioned installs paths with matching pkginfo version.
    pub fn matching(&self, pkginfo: &Dictionary) -> Result<Vec<Dictionary>, String> {
        let Some(hash) = text(pkginfo, "installer_item_hash").filter(|s| !s.is_empty()) else {
            return Ok(Vec::new());
        };
        if let Some(indexes) = self.hashes.get(hash) {
            return Ok(self.selected(indexes.iter().copied()));
        }
        let applications: Vec<_> = entries(pkginfo, "installs")
            .filter(|i| {
                matches!(text(i, "type"), Some("application" | "bundle"))
                    && text(i, "path").is_some()
            })
            .collect();
        if !applications.is_empty() {
            let mut indexes = BTreeSet::new();
            for application in applications {
                let path = text(application, "path").unwrap();
                let version = version(application).ok_or_else(|| {
                    format!("Application installs item {path} lacks its comparison version")
                })?;
                let Some(found) = self.applications.get(path).and_then(|v| v.get(version)) else {
                    return Ok(Vec::new());
                };
                intersection(&mut indexes, found);
            }
            if !indexes.is_empty() {
                return Ok(self.selected(indexes));
            }
        }
        let mut indexes = BTreeSet::new();
        for receipt in entries(pkginfo, "receipts") {
            let (Some(id), Some(version)) = (
                text(receipt, "packageid").filter(|s| !s.is_empty()),
                text(receipt, "version").filter(|s| !s.is_empty()),
            ) else {
                continue;
            };
            let Some(found) = self.receipts.get(id).and_then(|v| v.get(version)) else {
                return Ok(Vec::new());
            };
            intersection(&mut indexes, found);
        }
        if !indexes.is_empty() {
            return Ok(self.selected(indexes));
        }
        for file in entries(pkginfo, "installs").filter(|i| text(i, "type") == Some("file")) {
            if let (Some(path), Some(hash)) = (text(file, "path"), text(file, "md5checksum")) {
                if let Some(found) = self
                    .checksums
                    .get(hash)
                    .and_then(|entries| entries.iter().find(|entry| entry.path == path))
                {
                    return Ok(self.selected([found.index]));
                }
            }
        }
        for file in entries(pkginfo, "installs")
            .filter(|i| text(i, "type") == Some("file") && !i.contains_key("md5checksum"))
        {
            if let Some(path) = text(file, "path") {
                if let Some(entries) = self.files.get(path) {
                    for found in entries {
                        let candidate = self.items[found.index].as_dictionary().unwrap();
                        if candidate.get("version") == pkginfo.get("version") {
                            return Ok(vec![candidate.clone()]);
                        }
                    }
                }
            }
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(name: &str, version: &str) -> Dictionary {
        Dictionary::from_iter([
            ("name", Value::String(name.into())),
            ("version", Value::String(version.into())),
        ])
    }
    fn receipt(id: &str, version: &str) -> Value {
        Value::Dictionary(Dictionary::from_iter([
            ("packageid", Value::String(id.into())),
            ("version", Value::String(version.into())),
        ]))
    }
    #[test]
    fn indexes_hashes_receipts_and_installer_basename() {
        let mut one = item("one", "1");
        one.insert("installer_item_hash".into(), "hash".into());
        one.insert("installer_item_location".into(), "folder/file.pkg".into());
        one.insert(
            "receipts".into(),
            Value::Array(vec![receipt("a", "1"), receipt("b", "2")]),
        );
        let mut two = item("two", "1");
        two.insert("receipts".into(), Value::Array(vec![receipt("a", "1")]));
        let db = CatalogIndex::new(vec![
            Value::Dictionary(Dictionary::new()),
            Value::Dictionary(one.clone()),
            Value::Dictionary(two),
        ])
        .unwrap();
        assert_eq!(db.hashes["hash"], vec![1]);
        assert_eq!(db.installer_items["file.pkg"]["1"], vec![1]);
        assert_eq!(db.matching(&one).unwrap(), vec![one.clone()]);
        one.insert("installer_item_hash".into(), "different".into());
        assert_eq!(
            db.matching(&one).unwrap()[0]["name"].as_string(),
            Some("one")
        );
        one.remove("installer_item_hash");
        assert!(db.matching(&one).unwrap().is_empty());
    }
    #[test]
    fn file_match_requires_path_and_path_only_requires_version() {
        let file = |path: &str, hash: Option<&str>| {
            let mut d = Dictionary::from_iter([
                ("type", "file".into()),
                ("path", Value::String(path.into())),
            ]);
            if let Some(hash) = hash {
                d.insert("md5checksum".into(), hash.into());
            }
            Value::Dictionary(d)
        };
        let mut one = item("one", "1");
        one.insert(
            "installs".into(),
            Value::Array(vec![file("/path", Some("abc")), file("/directory", None)]),
        );
        let db = CatalogIndex::new(vec![Value::Dictionary(one)]).unwrap();
        let mut query = item("new", "1");
        query.insert("installer_item_hash".into(), "new".into());
        query.insert(
            "installs".into(),
            Value::Array(vec![file("/wrong", Some("abc"))]),
        );
        assert!(db.matching(&query).unwrap().is_empty());
        query.insert(
            "installs".into(),
            Value::Array(vec![file("/path", Some("abc"))]),
        );
        assert_eq!(db.matching(&query).unwrap().len(), 1);
        query.insert(
            "installs".into(),
            Value::Array(vec![file("/directory", None)]),
        );
        assert_eq!(db.matching(&query).unwrap().len(), 1);
        query.insert("version".into(), "2".into());
        assert!(db.matching(&query).unwrap().is_empty());
    }
    /// `catalog-reference.plist` holds the pinned c36e58f result for these
    /// items: `AutoPkgLib.make_catalog_db()` as `database`, and
    /// `MunkiImporter._find_matching_pkginfo` for the query below as `matches`.
    #[test]
    fn pinned_reference_catalog_and_matching() {
        let temp = tempfile::tempdir().unwrap();
        let rust_repo = temp.path().join("rust");
        std::fs::create_dir_all(rust_repo.join("catalogs")).unwrap();
        let mut one = item("one", "1");
        one.insert("installer_item_hash".into(), "hash".into());
        one.insert("installer_item_location".into(), "folder/item.pkg".into());
        one.insert(
            "receipts".into(),
            Value::Array(vec![receipt("a", "1"), receipt("b", "2")]),
        );
        let application = Value::Dictionary(Dictionary::from_iter([
            ("type", Value::String("application".into())),
            ("path", "/Applications/Test.app".into()),
            ("CFBundleShortVersionString", "2.0".into()),
        ]));
        let checksum = Value::Dictionary(Dictionary::from_iter([
            ("type", Value::String("file".into())),
            ("path", "/file".into()),
            ("md5checksum", "abc".into()),
        ]));
        let directory = Value::Dictionary(Dictionary::from_iter([
            ("type", Value::String("file".into())),
            ("path", "/directory".into()),
        ]));
        one.insert(
            "installs".into(),
            Value::Array(vec![application, checksum, directory]),
        );
        let mut two = item("two", "2");
        two.insert("receipts".into(), Value::Array(vec![receipt("a", "1")]));
        let items = vec![
            Value::Dictionary(Dictionary::new()),
            Value::Dictionary(one.clone()),
            Value::Dictionary(two),
        ];
        Value::Array(items)
            .to_file_xml(rust_repo.join("catalogs/all"))
            .unwrap();
        one.insert("installer_item_hash".into(), "different".into());
        let expected = Value::from_reader(std::io::Cursor::new(
            include_bytes!("../tests/fixtures/catalog-reference.plist").as_slice(),
        ))
        .unwrap();
        let expected = expected.as_dictionary().unwrap();
        let db = crate::FileRepo::new(rust_repo).index().unwrap();
        assert_eq!(db.to_plist(), expected["database"]);
        assert_eq!(
            Value::Array(
                db.matching(&one)
                    .unwrap()
                    .into_iter()
                    .map(Value::Dictionary)
                    .collect()
            ),
            expected["matches"]
        );
    }
}
