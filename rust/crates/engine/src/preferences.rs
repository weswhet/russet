//! Explicit preference-file loading and overlays. macOS domain lookup belongs
//! to platform services; reading a plist file is not a CFPreferences substitute.
use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Plist,
    Json,
}
#[derive(Clone, Debug)]
pub struct Preferences {
    pub values: Dictionary,
    pub path: PathBuf,
    pub format: Format,
}

impl Preferences {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let (value, format) = match Value::from_reader(std::io::Cursor::new(&bytes)) {
            Ok(v) => (v, Format::Plist),
            Err(_) => {
                let json: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                (
                    crate::yaml_value(serde_yaml::to_value(json).map_err(|e| e.to_string())?)?,
                    Format::Json,
                )
            }
        };
        Ok(Self {
            values: value
                .into_dictionary()
                .ok_or("Preferences must be a dictionary")?,
            path: path.to_owned(),
            format,
        })
    }
    /// Match the reference's file ordering: config.plist precedes config.json.
    /// A malformed existing file is reported rather than silently discarded.
    pub fn load_directory(directory: &Path) -> Result<Option<Self>, String> {
        for name in ["config.plist", "config.json"] {
            let path = directory.join(name);
            if path.exists() {
                let prefs = Self::load(&path)?;
                if !prefs.values.is_empty() {
                    return Ok(Some(prefs));
                }
            }
        }
        Ok(None)
    }
    pub fn overlay(&mut self, path: &Path) -> Result<(), String> {
        let other = Self::load(path)?;
        self.values.extend(other.values);
        self.path = other.path;
        self.format = other.format;
        Ok(())
    }
    pub fn save(&self) -> Result<(), String> {
        match self.format {
            Format::Plist => Value::Dictionary(self.values.clone())
                .to_file_xml(&self.path)
                .map_err(|e| e.to_string()),
            Format::Json => {
                let bytes =
                    serde_json::to_vec_pretty(&to_json(&Value::Dictionary(self.values.clone()))?)
                        .map_err(|e| e.to_string())?;
                std::fs::write(&self.path, bytes).map_err(|e| e.to_string())
            }
        }
    }
}
fn to_json(value: &Value) -> Result<serde_json::Value, String> {
    use serde_json::Value as J;
    Ok(match value {
        Value::Null => J::Null,
        Value::String(v) => J::String(v.clone()),
        Value::Boolean(v) => J::Bool(*v),
        Value::Integer(v) => {
            if let Some(n) = v.as_signed() {
                J::Number(n.into())
            } else {
                J::Number(v.as_unsigned().unwrap().into())
            }
        }
        Value::Real(v) => J::Number(
            serde_json::Number::from_f64(*v)
                .ok_or("Nonfinite numbers cannot be written to JSON preferences")?,
        ),
        Value::Array(v) => J::Array(v.iter().map(to_json).collect::<Result<_, _>>()?),
        Value::Dictionary(v) => J::Object(
            v.iter()
                .map(|(k, v)| Ok((k.clone(), to_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
        _ => {
            return Err(
                "Dates and binary data cannot be written to JSON preferences without losing types"
                    .into(),
            )
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_preferences_preserve_null() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        std::fs::write(&path, r#"{"optional":null,"nested":[null,""]}"#).unwrap();
        let prefs = Preferences::load(&path).unwrap();
        assert!(prefs.values["optional"].is_null());
        prefs.save().unwrap();
        assert_eq!(Preferences::load(&path).unwrap().values, prefs.values);
    }
    #[test]
    fn overlays_and_preserves_file_format() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        std::fs::write(&path, r#"{"NAME":"old","flag":true,"paths":["one"]}"#).unwrap();
        let mut prefs = Preferences::load(&path).unwrap();
        let overlay = temp.path().join("override.plist");
        Value::Dictionary(Dictionary::from_iter([(
            "NAME",
            Value::String("new".into()),
        )]))
        .to_file_binary(&overlay)
        .unwrap();
        prefs.overlay(&overlay).unwrap();
        assert_eq!(prefs.values["NAME"].as_string(), Some("new"));
        assert_eq!(prefs.values["flag"].as_boolean(), Some(true));
        assert_eq!(prefs.format, Format::Plist);
        prefs.save().unwrap();
        assert_eq!(Preferences::load(&overlay).unwrap().values, prefs.values);
    }
    #[test]
    fn plist_precedes_json() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("config.json"), r#"{"NAME":"json"}"#).unwrap();
        Value::Dictionary(Dictionary::from_iter([(
            "NAME",
            Value::String("plist".into()),
        )]))
        .to_file_xml(temp.path().join("config.plist"))
        .unwrap();
        assert_eq!(
            Preferences::load_directory(temp.path())
                .unwrap()
                .unwrap()
                .values["NAME"]
                .as_string(),
            Some("plist")
        );
    }
}
