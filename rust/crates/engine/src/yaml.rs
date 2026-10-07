//! AutoPkg's YAML 1.1 scalar schema; implicit floats intentionally remain strings.
use base64::Engine;
use chrono::{NaiveDate, TimeZone, Utc};
use plist::{Dictionary, Value};
use regex::Regex;
use std::{collections::HashMap, sync::LazyLock};
use yaml_rust2::{
    parser::{Event, Parser, Tag},
    scanner::TScalarStyle,
};

pub fn parse(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut parser = Parser::new_from_str(text);
    let mut events = Vec::new();
    loop {
        let (event, _) = parser.next_token().map_err(|e| e.to_string())?;
        let done = event == Event::StreamEnd;
        events.push(event);
        if done {
            break;
        }
    }
    let mut reader = Reader {
        events: events.into_iter().peekable(),
        anchors: HashMap::new(),
    };
    if reader.events.next() != Some(Event::StreamStart)
        || reader.events.next() != Some(Event::DocumentStart)
    {
        return Err("YAML recipe must contain one document".into());
    }
    let value = reader.node(0)?;
    if reader.events.next() != Some(Event::DocumentEnd)
        || reader.events.next() != Some(Event::StreamEnd)
    {
        return Err("YAML recipe must contain exactly one document".into());
    }
    Ok(value)
}
struct Reader {
    events: std::iter::Peekable<std::vec::IntoIter<Event>>,
    anchors: HashMap<usize, Value>,
}
fn tag_name(tag: Option<Tag>) -> Result<Option<String>, String> {
    tag.map(|tag| {
        if tag.handle == "tag:yaml.org,2002:" {
            Ok(tag.suffix)
        } else {
            Err(format!("Unsupported YAML tag {}{}", tag.handle, tag.suffix))
        }
    })
    .transpose()
}
impl Reader {
    fn node(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err("YAML nesting exceeds 128 levels".into());
        }
        let (anchor, value) = match self.events.next().ok_or("Unexpected end of YAML")? {
            Event::Alias(id) => {
                return self
                    .anchors
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| "Recursive YAML aliases are unsupported".into())
            }
            Event::Scalar(s, style, anchor, tag) => {
                (anchor, scalar(&s, style, tag_name(tag)?.as_deref())?)
            }
            Event::SequenceStart(anchor, tag) => {
                if tag_name(tag)?.is_some_and(|t| t != "seq") {
                    return Err("Unsupported YAML sequence tag".into());
                }
                let mut values = Vec::new();
                while self.events.peek() != Some(&Event::SequenceEnd) {
                    values.push(self.node(depth + 1)?);
                }
                self.events.next();
                (anchor, Value::Array(values))
            }
            Event::MappingStart(anchor, tag) => {
                if tag_name(tag)?.is_some_and(|t| t != "map") {
                    return Err("Unsupported YAML mapping tag".into());
                }
                let mut merged = Dictionary::new();
                let mut explicit = Dictionary::new();
                while self.events.peek() != Some(&Event::MappingEnd) {
                    let merge = matches!(self.events.peek(), Some(Event::Scalar(s, TScalarStyle::Plain, _, None)) if s == "<<")
                        || matches!(self.events.peek(), Some(Event::Scalar(_, _, _, Some(t))) if t.handle == "tag:yaml.org,2002:" && t.suffix == "merge");
                    if merge {
                        self.events.next();
                    }
                    let key = if merge {
                        String::new()
                    } else {
                        self.node(depth + 1)?
                            .into_string()
                            .ok_or("Recipe dictionary keys must be strings")?
                    };
                    let value = self.node(depth + 1)?;
                    if merge {
                        let maps = match value {
                            Value::Array(v) => v,
                            v => vec![v],
                        };
                        for map in maps {
                            for (k, v) in map
                                .into_dictionary()
                                .ok_or("YAML merge requires dictionaries")?
                            {
                                if !merged.contains_key(&k) {
                                    merged.insert(k, v);
                                }
                            }
                        }
                    } else {
                        explicit.insert(key, value);
                    }
                }
                self.events.next();
                merged.extend(explicit);
                (anchor, Value::Dictionary(merged))
            }
            e => return Err(format!("Unexpected YAML event {e:?}")),
        };
        if anchor != 0 {
            self.anchors.insert(anchor, value.clone());
        }
        Ok(value)
    }
}
fn integer(s: &str) -> Result<Value, String> {
    let clean = s.replace('_', "");
    let negative = clean.starts_with('-');
    let digits = clean.trim_start_matches(['-', '+']);
    let n = if digits.contains(':') {
        digits.split(':').try_fold(0i128, |n, part| {
            n.checked_mul(60)
                .and_then(|n| part.parse::<i128>().ok().and_then(|p| n.checked_add(p)))
                .ok_or("YAML integer out of range")
        })?
    } else {
        let (radix, digits) = if let Some(d) = digits.strip_prefix("0b") {
            (2, d)
        } else if let Some(d) = digits.strip_prefix("0x") {
            (16, d)
        } else if digits.starts_with('0') && digits.len() > 1 {
            (8, &digits[1..])
        } else {
            (10, digits)
        };
        i128::from_str_radix(digits, radix).map_err(|e| format!("Invalid YAML integer: {e}"))?
    };
    let n = if negative { -n } else { n };
    if let Ok(n) = i64::try_from(n) {
        Ok(Value::Integer(n.into()))
    } else if let Ok(n) = u64::try_from(n) {
        Ok(Value::Integer(n.into()))
    } else {
        Err("YAML integer exceeds native plist range".into())
    }
}
fn timestamp(s: &str) -> Result<Value, String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})(?:[Tt \t]+([0-9]{1,2}):([0-9]{2}):([0-9]{2})(?:\.([0-9]*))?(?:[ \t]*(Z|([-+])([0-9]{1,2})(?::([0-9]{2}))?))?)?$").unwrap()
    });
    let c = RE.captures(s).ok_or("Invalid YAML timestamp")?;
    let num = |i: usize| {
        c.get(i).map_or(Ok(0), |m| {
            m.as_str()
                .parse::<u32>()
                .map_err(|_| "Invalid timestamp component")
        })
    };
    if num(1)? == 0 {
        return Err("Invalid YAML year zero".into());
    }
    let date =
        NaiveDate::from_ymd_opt(num(1)? as i32, num(2)?, num(3)?).ok_or("Invalid YAML date")?;
    let micros = c
        .get(7)
        .map(|m| {
            format!("{:0<6}", &m.as_str()[..m.as_str().len().min(6)])
                .parse::<u32>()
                .unwrap()
        })
        .unwrap_or(0);
    let date = date
        .and_hms_micro_opt(num(4)?, num(5)?, num(6)?, micros)
        .ok_or("Invalid YAML time")?;
    let offset = (num(10)? * 3600 + num(11)? * 60) as i64
        * if c.get(9).is_some_and(|m| m.as_str() == "-") {
            -1
        } else {
            1
        };
    if offset.abs() >= 86400 {
        return Err("Invalid YAML timezone".into());
    }
    let utc = Utc.from_utc_datetime(&date) - chrono::Duration::seconds(offset);
    let system: std::time::SystemTime = utc.into();
    Ok(Value::Date(system.into()))
}
fn scalar(s: &str, style: TScalarStyle, tag: Option<&str>) -> Result<Value, String> {
    let implicit = tag.is_none() && style == TScalarStyle::Plain;
    let bool_source = if tag == Some("bool") {
        s.to_lowercase()
    } else {
        s.to_owned()
    };
    let bool_value = match bool_source.as_str() {
        "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => Some(true),
        "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => Some(false),
        _ => None,
    };
    if tag == Some("null") || (implicit && matches!(s, "" | "~" | "null" | "Null" | "NULL")) {
        return Ok(Value::Null);
    }
    if tag == Some("bool") || (implicit && bool_value.is_some()) {
        return bool_value
            .map(Value::Boolean)
            .ok_or_else(|| "Invalid YAML boolean".into());
    }
    static INT_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[-+]?(?:0b[01_]+|0[0-7_]+|0|[1-9][0-9_]*|0x[0-9a-fA-F_]+|[1-9][0-9_]*(?::[0-5]?[0-9])+)$").unwrap()
    });
    if tag == Some("int") || (implicit && INT_RE.is_match(s)) {
        return integer(s);
    }
    static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}[Tt \t]+[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?)$").unwrap()
    });
    if tag == Some("timestamp") || (implicit && DATE_RE.is_match(s)) {
        return timestamp(s);
    }
    match tag {
        Some("binary") => {
            if !s.is_ascii() {
                return Err("YAML binary must contain only ASCII characters".into());
            }
            // PyYAML's base64.decodebytes ignores non-alphabet ASCII bytes.
            let clean: String = s
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
                .collect();
            let engine = base64::engine::GeneralPurpose::new(
                &base64::alphabet::STANDARD,
                base64::engine::general_purpose::GeneralPurposeConfig::new()
                    .with_decode_allow_trailing_bits(true),
            );
            Ok(Value::Data(
                engine
                    .decode(clean)
                    .map_err(|e| format!("Invalid YAML binary: {e}"))?,
            ))
        }
        Some("float") => {
            let clean = s.replace('_', "").to_lowercase();
            let sign = if clean.starts_with('-') { -1.0 } else { 1.0 };
            let unsigned = clean.trim_start_matches(['-', '+']);
            let n = match unsigned {
                ".inf" => sign * f64::INFINITY,
                ".nan" => f64::NAN,
                _ if unsigned.contains(':') => {
                    sign * unsigned
                        .split(':')
                        .try_fold(0.0, |n, part| part.parse::<f64>().map(|p| n * 60.0 + p))
                        .map_err(|e| e.to_string())?
                }
                _ => clean
                    .parse::<f64>()
                    .map_err(|e| format!("Invalid YAML float: {e}"))?,
            };
            Ok(Value::Real(n))
        }
        None | Some("str") => Ok(Value::String(s.into())),
        Some(t) => Err(format!("Unsupported YAML tag {t}")),
    }
}

/// Emit explicit standard tags for types AutoPkg cannot infer from plain YAML.
/// JSON-style strings preserve quoting and avoid YAML 1.1 implicit bool/int rules.
pub(super) fn serialize(value: &Value) -> Result<String, String> {
    fn emit(value: &Value, depth: usize) -> Result<String, String> {
        if depth > 128 {
            return Err("YAML nesting exceeds 128 levels".into());
        }
        let quoted = |s: &str| serde_json::to_string(s).map_err(|e| e.to_string());
        Ok(match value {
            Value::Null => "null".into(),
            Value::String(s) => quoted(s)?,
            Value::Boolean(b) => b.to_string(),
            Value::Integer(n) => n.to_string(),
            Value::Real(n) => {
                let text = if n.is_nan() {
                    ".nan".into()
                } else if *n == f64::INFINITY {
                    ".inf".into()
                } else if *n == f64::NEG_INFINITY {
                    "-.inf".into()
                } else {
                    n.to_string()
                };
                format!("!!float {}", quoted(&text)?)
            }
            Value::Date(d) => format!("!!timestamp {}", quoted(&d.to_xml_format())?),
            Value::Data(bytes) => format!(
                "!!binary {}",
                quoted(&base64::engine::general_purpose::STANDARD.encode(bytes))?
            ),
            Value::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| emit(v, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ")
            ),
            Value::Dictionary(values) => {
                if values.is_empty() {
                    "{}".into()
                } else {
                    let indent = "  ".repeat(depth + 1);
                    let entries = values
                        .iter()
                        .map(|(k, v)| {
                            Ok(format!("{indent}{}: {}", quoted(k)?, emit(v, depth + 1)?))
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    format!("{{\n{}\n{}}}", entries.join(",\n"), "  ".repeat(depth))
                }
            }
            Value::Uid(_) => {
                return Err("UID values cannot be represented in AutoPkg YAML recipes".into())
            }
        })
    }
    Ok(format!("{}\n", emit(value, 0)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = r#"
Identifier: org.autopkg.yaml-types
Input:
  null_value: null
  null_array: [null, ""]
  quoted_bool: "yes"
  bools: [yes, No, ON, false]
  mixed_case: yEs
  integer: 42
  negative: -17
  octal: 0755
  hex: 0xff
  binary_int: 0b1010
  sexagesimal: 1:20:30
  underscored: 1_000
  quoted_integer: '0755'
  invalid_octal: 08
  version: 2.3
  exponent: 1e3
  infinity_string: .inf
  explicit_float: !!float 2.3
  float_sexagesimal: !!float -1:20.5
  tagged_string: !!str yes
  date: 2024-02-29
  timestamp: 2024-2-29 3:04:05.123456789 -7:30
  naive_timestamp: 2024-02-29t03:04:05
  quoted_timestamp: "2024-02-29"
  ignored_binary_chars: !!binary "S!G VsbG8="
  explicit_mixed_bool: !!bool yEs
  data: !!binary |
    SGVsbG8A
    /w==
  literal: |
    yes
    0755
  defaults: &defaults {first: one, second: two}
  alternate: &alternate {first: alternate, third: three}
  merged: {<<: [*defaults, *alternate], second: explicit}
  alias: *defaults
  quoted_merge: {"<<": literal}
Process: []
"#;
    #[test]
    fn typed_yaml_output_round_trip() {
        let source = parse(FIXTURE.as_bytes()).unwrap();
        let emitted = serialize(&source).unwrap();
        assert_eq!(parse(emitted.as_bytes()).unwrap(), source);
        assert!(emitted.contains("!!timestamp"));
        assert!(emitted.contains("!!binary"));
        assert!(emitted.contains("!!float"));
    }
    #[test]
    fn null_values_remain_distinct_from_strings() {
        let value = parse(
            b"plain: null\nexplicit: !!null ignored\nquoted: 'null'\nitems: [~, null, '', false]\n",
        )
        .unwrap();
        let d = value.as_dictionary().unwrap();
        assert!(d["plain"].is_null());
        assert!(d["explicit"].is_null());
        assert_eq!(d["quoted"].as_string(), Some("null"));
        assert!(d["items"].as_array().unwrap()[0].is_null());
        assert_eq!(d["items"].as_array().unwrap()[2].as_string(), Some(""));
    }
    #[test]
    fn reference_scalar_types_and_aliases() {
        let root = parse(FIXTURE.as_bytes())
            .unwrap()
            .into_dictionary()
            .unwrap();
        let d = root["Input"].as_dictionary().unwrap();
        for (key, expected) in [
            ("quoted_bool", "yes"),
            ("mixed_case", "yEs"),
            ("quoted_integer", "0755"),
            ("invalid_octal", "08"),
            ("version", "2.3"),
            ("exponent", "1e3"),
            ("infinity_string", ".inf"),
            ("tagged_string", "yes"),
            ("quoted_timestamp", "2024-02-29"),
            ("literal", "yes\n0755\n"),
        ] {
            assert_eq!(d[key].as_string(), Some(expected), "{key}");
        }
        for (key, expected) in [
            ("integer", 42),
            ("negative", -17),
            ("octal", 493),
            ("hex", 255),
            ("binary_int", 10),
            ("sexagesimal", 4830),
            ("underscored", 1000),
        ] {
            assert_eq!(d[key].as_signed_integer(), Some(expected), "{key}");
        }
        assert_eq!(
            d["bools"],
            Value::Array(vec![true.into(), false.into(), true.into(), false.into()])
        );
        assert_eq!(d["explicit_float"].as_real(), Some(2.3));
        assert_eq!(d["float_sexagesimal"].as_real(), Some(-80.5));
        assert_eq!(d["data"].as_data(), Some(b"Hello\0\xff".as_slice()));
        assert_eq!(
            d["date"].as_date().unwrap().to_xml_format(),
            "2024-02-29T00:00:00Z"
        );
        assert_eq!(
            d["timestamp"].as_date().unwrap().to_xml_format(),
            "2024-02-29T10:34:05.123456Z"
        );
        assert_eq!(
            d["naive_timestamp"].as_date().unwrap().to_xml_format(),
            "2024-02-29T03:04:05Z"
        );
        assert_eq!(d["defaults"], d["alias"]);
        let merged = d["merged"].as_dictionary().unwrap();
        assert_eq!(merged["first"].as_string(), Some("one"));
        assert_eq!(merged["second"].as_string(), Some("explicit"));
        assert_eq!(merged["third"].as_string(), Some("three"));
        assert_eq!(
            d["quoted_merge"].as_dictionary().unwrap()["<<"].as_string(),
            Some("literal")
        );
    }
    #[test]
    fn rejects_unrepresentable_or_invalid_yaml() {
        for source in [
            "a: !!python/object:thing {}",
            "a: !!int wrong",
            "a: 2024-02-30",
            "a: 0000-01-01",
            "a: !!timestamp 2024-01-01T01:02:03.١٢٣٤",
            "a: &cycle [*cycle]",
            "a: !!binary 'A'",
            "a: 18446744073709551616",
            "a: one\n---\na: two",
        ] {
            assert!(parse(source.as_bytes()).is_err(), "{source}");
        }
        assert_eq!(
            parse(b"a: 'null'").unwrap().as_dictionary().unwrap()["a"].as_string(),
            Some("null")
        );
    }
    /// `yaml-typed-reference.bplist` holds the pinned c36e58f result for
    /// FIXTURE: `AutoPkgYAMLLoader` output, with aware timestamps converted to
    /// naive UTC and dates widened to midnight, passed through
    /// `plist_serializer` and written as a binary plist. When it was captured,
    /// the pinned loader also read this module's serialized FIXTURE back to the
    /// same value.
    #[test]
    fn pinned_reference_typed_fixture() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("typed.recipe.yaml");
        std::fs::write(&input, FIXTURE).unwrap();
        let emitted = directory.path().join("emitted.recipe.yaml");
        std::fs::write(
            &emitted,
            serialize(&parse(FIXTURE.as_bytes()).unwrap()).unwrap(),
        )
        .unwrap();
        let expected = Value::from_reader(std::io::Cursor::new(
            include_bytes!("../tests/fixtures/yaml-typed-reference.bplist").as_slice(),
        ))
        .unwrap();
        // Binary plist dates use floating-point seconds; compare date instants
        // within that encoding's resolution, retaining all other value types.
        fn compare(a: &Value, b: &Value) {
            match (a, b) {
                (Value::Null, Value::String(s)) => assert!(s.is_empty()),
                (Value::Date(a), Value::Date(b)) => {
                    let a: std::time::SystemTime = (*a).into();
                    let b: std::time::SystemTime = (*b).into();
                    assert!(
                        a.duration_since(b)
                            .or_else(|_| b.duration_since(a))
                            .unwrap()
                            .as_nanos()
                            < 1000
                    );
                }
                (Value::Dictionary(a), Value::Dictionary(b)) => {
                    assert_eq!(a.len(), b.len());
                    for (k, v) in a {
                        compare(v, &b[k]);
                    }
                }
                (Value::Array(a), Value::Array(b)) => {
                    assert_eq!(a.len(), b.len());
                    for (a, b) in a.iter().zip(b) {
                        compare(a, b);
                    }
                }
                _ => assert_eq!(a, b),
            }
        }
        for path in [&input, &emitted] {
            compare(
                &Value::Dictionary(crate::read_recipe(path).unwrap()),
                &expected,
            );
        }
    }
}
