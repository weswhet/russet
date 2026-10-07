//! Recipe values preserve null until an explicit property-list boundary.
//! Python's plist_serializer maps nested None values to empty strings; JSON
//! and YAML serialization retain null. No reserved-string sentinel is used.
pub use native_plist::{Date, Integer, Uid};
mod binary;
#[derive(Debug)]
pub enum Error {
    Native(native_plist::Error),
    Io(std::io::Error),
    Binary(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
            Self::Binary(e) => f.write_str(e),
        }
    }
}
impl std::error::Error for Error {}
impl From<native_plist::Error> for Error {
    fn from(e: native_plist::Error) -> Self {
        Self::Native(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Seek, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Array(Vec<Value>),
    Dictionary(Dictionary),
    Boolean(bool),
    Data(Vec<u8>),
    Date(Date),
    Real(f64),
    Integer(Integer),
    String(String),
    Uid(Uid),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dictionary(indexmap::IndexMap<String, Value>);
impl Dictionary {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.0.insert(key, value)
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.0.get_mut(key)
    }
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.0.shift_remove(key)
    }
    pub fn entry<S: Into<String>>(&mut self, key: S) -> indexmap::map::Entry<'_, String, Value> {
        self.0.entry(key.into())
    }
    pub fn sort_keys(&mut self) {
        self.0.sort_keys();
    }
}
impl std::ops::Deref for Dictionary {
    type Target = indexmap::IndexMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Dictionary {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Dictionary {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}
impl<K: Into<String>, V: Into<Value>> Extend<(K, V)> for Dictionary {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, iter: T) {
        self.0
            .extend(iter.into_iter().map(|(k, v)| (k.into(), v.into())));
    }
}
impl IntoIterator for Dictionary {
    type Item = (String, Value);
    type IntoIter = indexmap::map::IntoIter<String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a Dictionary {
    type Item = (&'a String, &'a Value);
    type IntoIter = indexmap::map::Iter<'a, String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl<'a> IntoIterator for &'a mut Dictionary {
    type Item = (&'a String, &'a mut Value);
    type IntoIter = indexmap::map::IterMut<'a, String, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        Self::from_reader(std::io::Cursor::new(std::fs::read(path)?))
    }
    pub fn from_reader<R: Read + Seek>(mut r: R) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes)?;
        if bytes.starts_with(b"bplist") {
            binary::parse(&bytes).map_err(Error::Binary)
        } else {
            native_plist::Value::from_reader(std::io::Cursor::new(bytes))
                .map(Self::from)
                .map_err(Into::into)
        }
    }
    pub fn from_reader_xml<R: Read>(r: R) -> Result<Self, Error> {
        native_plist::Value::from_reader_xml(r)
            .map(Self::from)
            .map_err(Into::into)
    }
    pub fn from_reader_ascii<R: Read>(r: R) -> Result<Self, Error> {
        native_plist::Value::from_reader_ascii(r)
            .map(Self::from)
            .map_err(Into::into)
    }
    pub fn to_file_xml<P: AsRef<Path>>(&self, p: P) -> Result<(), Error> {
        self.to_plist().to_file_xml(p).map_err(Into::into)
    }
    pub fn to_file_binary<P: AsRef<Path>>(&self, p: P) -> Result<(), Error> {
        self.to_plist().to_file_binary(p).map_err(Into::into)
    }
    pub fn to_writer_xml<W: Write>(&self, w: W) -> Result<(), Error> {
        self.to_plist().to_writer_xml(w).map_err(Into::into)
    }
    pub fn to_writer_binary<W: Write>(&self, w: W) -> Result<(), Error> {
        self.to_plist().to_writer_binary(w).map_err(Into::into)
    }
    /// Apply the reference plist_serializer's None-to-empty-string policy.
    pub fn to_plist(&self) -> native_plist::Value {
        match self {
            Self::Null => native_plist::Value::String(String::new()),
            Self::Array(v) => native_plist::Value::Array(v.iter().map(Self::to_plist).collect()),
            Self::Dictionary(v) => native_plist::Value::Dictionary(
                v.iter().map(|(k, v)| (k.clone(), v.to_plist())).collect(),
            ),
            Self::Boolean(v) => native_plist::Value::Boolean(*v),
            Self::Data(v) => native_plist::Value::Data(v.clone()),
            Self::Date(v) => native_plist::Value::Date(*v),
            Self::Real(v) => native_plist::Value::Real(*v),
            Self::Integer(v) => native_plist::Value::Integer(*v),
            Self::String(v) => native_plist::Value::String(v.clone()),
            Self::Uid(v) => native_plist::Value::Uid(*v),
        }
    }
    pub fn as_boolean(&self) -> Option<bool> {
        if let Self::Boolean(v) = self {
            Some(*v)
        } else {
            None
        }
    }
    pub fn as_date(&self) -> Option<Date> {
        if let Self::Date(v) = self {
            Some(*v)
        } else {
            None
        }
    }
    pub fn as_real(&self) -> Option<f64> {
        if let Self::Real(v) = self {
            Some(*v)
        } else {
            None
        }
    }
    pub fn as_signed_integer(&self) -> Option<i64> {
        if let Self::Integer(v) = self {
            v.as_signed()
        } else {
            None
        }
    }
    pub fn as_unsigned_integer(&self) -> Option<u64> {
        if let Self::Integer(v) = self {
            v.as_unsigned()
        } else {
            None
        }
    }
    pub fn as_string(&self) -> Option<&str> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_data(&self) -> Option<&[u8]> {
        if let Self::Data(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_uid(&self) -> Option<&Uid> {
        if let Self::Uid(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl From<native_plist::Value> for Value {
    fn from(v: native_plist::Value) -> Self {
        match v {
            native_plist::Value::Array(v) => Self::Array(v.into_iter().map(Self::from).collect()),
            native_plist::Value::Dictionary(v) => {
                Self::Dictionary(v.into_iter().map(|(k, v)| (k, Self::from(v))).collect())
            }
            native_plist::Value::Boolean(v) => Self::Boolean(v),
            native_plist::Value::Data(v) => Self::Data(v),
            native_plist::Value::Date(v) => Self::Date(v),
            native_plist::Value::Real(v) => Self::Real(v),
            native_plist::Value::Integer(v) => Self::Integer(v),
            native_plist::Value::String(v) => Self::String(v),
            native_plist::Value::Uid(v) => Self::Uid(v),
            _ => unreachable!("unrecognized native plist value variant"),
        }
    }
}
impl Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => s.serialize_none(),
            Self::Array(v) => v.serialize(s),
            Self::Dictionary(v) => v.serialize(s),
            Self::Boolean(v) => s.serialize_bool(*v),
            Self::Data(v) => s.serialize_bytes(v),
            Self::Date(v) => v.serialize(s),
            Self::Real(v) => s.serialize_f64(*v),
            Self::Integer(v) => v.serialize(s),
            Self::String(v) => s.serialize_str(v),
            Self::Uid(v) => v.serialize(s),
        }
    }
}
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a recipe value")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
                Ok(v.into())
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Value, E> {
                Ok(Value::Data(v.to_vec()))
            }
            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Value, E> {
                Ok(Value::Data(v))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element()? {
                    v.push(x)
                }
                Ok(Value::Array(v))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
                let mut v = Dictionary::new();
                while let Some((k, x)) = a.next_entry()? {
                    v.insert(k, x);
                }
                Ok(Value::Dictionary(v))
            }
        }
        d.deserialize_any(Visitor)
    }
}
impl Value {
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn as_dictionary(&self) -> Option<&Dictionary> {
        if let Self::Dictionary(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_dictionary_mut(&mut self) -> Option<&mut Dictionary> {
        if let Self::Dictionary(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn into_array(self) -> Option<Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn into_dictionary(self) -> Option<Dictionary> {
        if let Self::Dictionary(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn into_string(self) -> Option<String> {
        if let Self::String(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn into_data(self) -> Option<Vec<u8>> {
        if let Self::Data(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl Value {
    pub fn into_uid(self) -> Option<Uid> {
        if let Self::Uid(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Self::Array(v)
    }
}
impl From<Dictionary> for Value {
    fn from(v: Dictionary) -> Self {
        Self::Dictionary(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Boolean(v)
    }
}
impl From<Vec<u8>> for Value {
    fn from(v: Vec<u8>) -> Self {
        Self::Data(v)
    }
}
impl From<Date> for Value {
    fn from(v: Date) -> Self {
        Self::Date(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Real(v)
    }
}
impl From<Integer> for Value {
    fn from(v: Integer) -> Self {
        Self::Integer(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<Uid> for Value {
    fn from(v: Uid) -> Self {
        Self::Uid(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Integer(v.into())
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Integer(v.into())
    }
}
impl From<i16> for Value {
    fn from(v: i16) -> Self {
        Self::Integer(v.into())
    }
}
impl From<i8> for Value {
    fn from(v: i8) -> Self {
        Self::Integer(v.into())
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self::Integer(v.into())
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Self::Integer(v.into())
    }
}
impl From<u16> for Value {
    fn from(v: u16) -> Self {
        Self::Integer(v.into())
    }
}
impl From<u8> for Value {
    fn from(v: u8) -> Self {
        Self::Integer(v.into())
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<()> for Value {
    fn from(_: ()) -> Self {
        Self::Null
    }
}
impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Self::Real(v.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_types_round_trip_binary_and_xml() {
        let mut d = Dictionary::from_iter([
            ("string", Value::String("__NULL__".into())),
            ("integer", Value::Integer(u64::MAX.into())),
            ("negative", Value::Integer(i64::MIN.into())),
            ("real", Value::Real(1.25)),
            ("boolean", Value::Boolean(false)),
            ("data", Value::Data(vec![0, 255, 42])),
            (
                "date",
                Value::Date(Date::from_xml_format("2024-02-29T03:04:05Z").unwrap()),
            ),
            (
                "nested",
                Value::Array(vec![Value::Dictionary(Dictionary::new())]),
            ),
        ]);
        let original = Value::Dictionary(d.clone());
        for binary in [false, true] {
            let mut bytes = Vec::new();
            if binary {
                original.to_writer_binary(&mut bytes).unwrap();
            } else {
                original.to_writer_xml(&mut bytes).unwrap();
            }
            assert_eq!(
                Value::from_reader(std::io::Cursor::new(bytes)).unwrap(),
                original
            );
        }
        d.insert("uid".into(), Value::Uid(Uid::new(123)));
        let original = Value::Dictionary(d);
        let mut bytes = Vec::new();
        original.to_writer_binary(&mut bytes).unwrap();
        assert_eq!(
            Value::from_reader(std::io::Cursor::new(bytes)).unwrap(),
            original
        );
    }
    #[test]
    fn null_survives_json_and_yaml_but_plist_matches_python_serializer() {
        let value = Value::Dictionary(Dictionary::from_iter([
            ("null", Value::Null),
            ("empty", Value::String(String::new())),
            (
                "array",
                Value::Array(vec![
                    Value::Null,
                    Value::Dictionary(Dictionary::from_iter([("child", Value::Null)])),
                ]),
            ),
        ]));
        assert_eq!(
            serde_json::from_str::<Value>(&serde_json::to_string(&value).unwrap()).unwrap(),
            value
        );
        assert_eq!(
            serde_yaml::from_str::<Value>(&serde_yaml::to_string(&value).unwrap()).unwrap(),
            value
        );
        for binary in [false, true] {
            let mut bytes = Vec::new();
            if binary {
                value.to_writer_binary(&mut bytes).unwrap();
            } else {
                value.to_writer_xml(&mut bytes).unwrap();
            }
            let decoded = Value::from_reader(std::io::Cursor::new(bytes))
                .unwrap()
                .into_dictionary()
                .unwrap();
            assert_eq!(decoded["null"].as_string(), Some(""));
            let array = decoded["array"].as_array().unwrap();
            assert_eq!(array[0].as_string(), Some(""));
            assert_eq!(
                array[1].as_dictionary().unwrap()["child"].as_string(),
                Some("")
            );
        }
        assert!(
            value.as_dictionary().unwrap()["null"].is_null(),
            "serialization cannot mutate the environment"
        );
    }
    #[test]
    fn dictionary_keeps_insertion_order_and_replacement_position() {
        let mut d = Dictionary::from_iter([("z", 1), ("a", 2), ("m", 3)]);
        d.insert("a".into(), 4.into());
        d.remove("z");
        assert_eq!(d.keys().map(String::as_str).collect::<Vec<_>>(), ["a", "m"]);
    }
}

mod python;
pub use python::{python_quote, python_repr, python_str};
