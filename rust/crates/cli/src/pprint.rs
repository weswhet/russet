//! Python's `pprint.pformat`, for output that must match AutoPkg's. Values from
//! a macOS preference domain are shown as PyObjC objects, the way AutoPkg's
//! `Preferences` class converts them.
use plist::Value;
use plist::{python_quote, python_repr};

const WIDTH: i64 = 80;

/// A Python object as `pprint` sees it.
pub(crate) enum Py {
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Py>),
    Dict(Vec<(String, Py)>),
    /// Any other object, by its `repr()`.
    Other(String),
}

impl Py {
    /// A value as Python's `plistlib` or `json` modules load it.
    pub(crate) fn python(value: &Value) -> Self {
        match value {
            Value::String(s) => Self::Str(s.clone()),
            Value::Data(bytes) => Self::Bytes(bytes.clone()),
            Value::Array(values) => Self::List(values.iter().map(Self::python).collect()),
            Value::Dictionary(values) => Self::Dict(
                values
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::python(v)))
                    .collect(),
            ),
            other => Self::Other(python_repr(other)),
        }
    }

    /// A value from a macOS domain after AutoPkg's `__deepconvert_objc`:
    /// numbers become `int`, arrays become lists of converted values, and
    /// dictionaries become dicts whose dictionary values become dicts.
    /// Everything else stays a PyObjC object.
    pub(crate) fn autopkg_domain(value: &Value) -> Self {
        match value {
            Value::Integer(_) | Value::Real(_) => Self::Other(python_repr(&autopkg_number(value))),
            Value::Array(values) => Self::List(values.iter().map(Self::autopkg_domain).collect()),
            Value::Dictionary(values) => Self::Dict(
                values
                    .iter()
                    .map(|(k, v)| {
                        let v = match v {
                            Value::Dictionary(inner) => Self::Dict(
                                inner
                                    .iter()
                                    .map(|(k, v)| (k.clone(), Self::objc(v)))
                                    .collect(),
                            ),
                            other => Self::objc(other),
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => Self::objc(other),
        }
    }

    /// An unconverted PyObjC object.
    fn objc(value: &Value) -> Self {
        match value {
            Value::String(s) => Self::Str(s.clone()),
            Value::Boolean(_) | Value::Integer(_) | Value::Real(_) => {
                Self::Other(python_repr(value))
            }
            other => Self::Other(description(other, 0)),
        }
    }

    fn repr(&self) -> String {
        match self {
            Self::Str(s) => python_quote(s),
            Self::Bytes(bytes) => python_repr(&Value::Data(bytes.clone())),
            Self::List(items) => format!(
                "[{}]",
                items.iter().map(Self::repr).collect::<Vec<_>>().join(", ")
            ),
            Self::Dict(entries) => format!(
                "{{{}}}",
                sorted(entries)
                    .iter()
                    .map(|(k, v)| format!("{}: {}", python_quote(k), v.repr()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Other(repr) => repr.clone(),
        }
    }
}

/// AutoPkg's `int()` of a number from a macOS domain, which truncates.
pub(crate) fn autopkg_number(value: &Value) -> Value {
    match value {
        Value::Real(n) if n.is_finite() => Value::Integer((n.trunc() as i64).into()),
        other => other.clone(),
    }
}

fn sorted(entries: &[(String, Py)]) -> Vec<&(String, Py)> {
    let mut entries: Vec<_> = entries.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn len(text: &str) -> i64 {
    text.chars().count() as i64
}

/// `pprint.pformat(value, indent=indent)`.
pub(crate) fn pformat(value: &Py, indent: usize) -> String {
    let mut printer = Printer {
        out: String::new(),
        per_level: indent as i64,
    };
    printer.format(value, 0, 0, 0);
    printer.out
}

struct Printer {
    out: String,
    per_level: i64,
}

impl Printer {
    fn spaces(&mut self, count: i64) {
        self.out.push_str(&" ".repeat(count.max(0) as usize));
    }

    fn format(&mut self, value: &Py, indent: i64, allowance: i64, level: usize) {
        let rep = value.repr();
        if len(&rep) > WIDTH - indent - allowance {
            match value {
                Py::Dict(entries) => return self.dict(entries, indent, allowance, level + 1),
                Py::List(items) => return self.list(items, indent, allowance, level + 1),
                Py::Str(text) => return self.string(text, indent, allowance, level + 1),
                Py::Bytes(bytes) => return self.bytes(bytes, indent, allowance, level + 1),
                _ => {}
            }
        }
        self.out.push_str(&rep);
    }

    fn dict(&mut self, entries: &[(String, Py)], indent: i64, allowance: i64, level: usize) {
        self.out.push('{');
        if self.per_level > 1 {
            self.spaces(self.per_level - 1);
        }
        let entries = sorted(entries);
        let indent = indent + self.per_level;
        for (i, (key, value)) in entries.iter().enumerate() {
            let last = i + 1 == entries.len();
            let rep = python_quote(key);
            self.out.push_str(&rep);
            self.out.push_str(": ");
            let allowance = if last { allowance + 1 } else { 1 };
            self.format(value, indent + len(&rep) + 2, allowance, level);
            if !last {
                self.out.push_str(",\n");
                self.spaces(indent);
            }
        }
        self.out.push('}');
    }

    fn list(&mut self, items: &[Py], indent: i64, allowance: i64, level: usize) {
        self.out.push('[');
        let indent = indent + self.per_level;
        if self.per_level > 1 && !items.is_empty() {
            self.spaces(self.per_level - 1);
        }
        for (i, item) in items.iter().enumerate() {
            let last = i + 1 == items.len();
            if i > 0 {
                self.out.push_str(",\n");
                self.spaces(indent);
            }
            self.format(item, indent, if last { allowance + 1 } else { 1 }, level);
        }
        self.out.push(']');
    }

    fn string(&mut self, text: &str, mut indent: i64, mut allowance: i64, level: usize) {
        if text.is_empty() {
            self.out.push_str(&python_quote(text));
            return;
        }
        if level == 1 {
            indent += 1;
            allowance += 1;
        }
        let lines = split_lines(text);
        let max_width = WIDTH - indent;
        let mut max_width1 = max_width;
        let mut chunks = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let rep = python_quote(line);
            let last_line = i + 1 == lines.len();
            if last_line {
                max_width1 -= allowance;
            }
            if len(&rep) <= max_width1 {
                chunks.push(rep);
                continue;
            }
            let parts = words(line);
            let mut max_width2 = max_width;
            let mut current = String::new();
            for (j, part) in parts.iter().enumerate() {
                let candidate = format!("{current}{part}");
                if j + 1 == parts.len() && last_line {
                    max_width2 -= allowance;
                }
                if len(&python_quote(&candidate)) > max_width2 {
                    if !current.is_empty() {
                        chunks.push(python_quote(&current));
                    }
                    current = (*part).to_owned();
                } else {
                    current = candidate;
                }
            }
            if !current.is_empty() {
                chunks.push(python_quote(&current));
            }
        }
        if chunks.len() == 1 {
            // Python writes the last line's repr here, which is the same text.
            self.out.push_str(&python_quote(lines.last().unwrap()));
            return;
        }
        if level == 1 {
            self.out.push('(');
        }
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
                self.spaces(indent);
            }
            self.out.push_str(chunk);
        }
        if level == 1 {
            self.out.push(')');
        }
    }

    fn bytes(&mut self, bytes: &[u8], mut indent: i64, mut allowance: i64, level: usize) {
        let repr = |b: &[u8]| python_repr(&Value::Data(b.to_vec()));
        if bytes.len() <= 4 {
            self.out.push_str(&repr(bytes));
            return;
        }
        let parens = level == 1;
        if parens {
            indent += 1;
            allowance += 1;
            self.out.push('(');
        }
        let mut width = WIDTH - indent;
        let last = bytes.len() / 4 * 4;
        let mut chunks = Vec::new();
        let mut current: Vec<u8> = Vec::new();
        for start in (0..bytes.len()).step_by(4) {
            let part = &bytes[start..(start + 4).min(bytes.len())];
            let candidate = [current.as_slice(), part].concat();
            if start == last {
                width -= allowance;
            }
            if len(&repr(&candidate)) > width {
                if !current.is_empty() {
                    chunks.push(repr(&current));
                }
                current = part.to_vec();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            chunks.push(repr(&current));
        }
        for (i, chunk) in chunks.iter().enumerate() {
            if i > 0 {
                self.out.push('\n');
                self.spaces(indent);
            }
            self.out.push_str(chunk);
        }
        if parens {
            self.out.push(')');
        }
    }
}

/// `str.splitlines(True)`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = match c {
            '\r' if chars.peek().is_some_and(|(_, next)| *next == '\n') => {
                chars.next();
                i + 2
            }
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}'
            | '\u{2029}' => i + c.len_utf8(),
            _ => continue,
        };
        lines.push(&text[start..end]);
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// `re.findall(r'\S*\s*', line)` without the final empty match.
fn words(line: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut seen_space = false;
    for (i, c) in line.char_indices() {
        if is_space(c) {
            seen_space = true;
        } else if seen_space {
            parts.push(&line[start..i]);
            start = i;
            seen_space = false;
        }
    }
    if start < line.len() {
        parts.push(&line[start..]);
    }
    parts
}

/// Foundation's `description` of a property list object, as PyObjC's
/// `repr()` shows it.
fn description(value: &Value, level: usize) -> String {
    let pad = |level: usize| "    ".repeat(level);
    match value {
        Value::Array(items) => {
            let rows: Vec<_> = items
                .iter()
                .map(|item| format!("{}{}", pad(level + 1), element(item, level + 1)))
                .collect();
            if rows.is_empty() {
                format!("{}(\n{})", pad(level), pad(level))
            } else {
                format!("{}(\n{}\n{})", pad(level), rows.join(",\n"), pad(level))
            }
        }
        Value::Dictionary(entries) => {
            let mut keys: Vec<_> = entries.keys().collect();
            keys.sort();
            let rows: String = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}{} = {};\n",
                        pad(level + 1),
                        quoted(key),
                        element(&entries[key], level + 1)
                    )
                })
                .collect();
            format!("{}{{\n{rows}{}}}", pad(level), pad(level))
        }
        Value::Date(date) => {
            let text = date.to_xml_format();
            format!(
                "{} +0000",
                text.trim_end_matches('Z')
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .replace('T', " ")
            )
        }
        Value::Data(bytes) => {
            let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02x}")).collect::<String>();
            let bytes_text = if bytes.len() <= 24 {
                hex(bytes)
            } else {
                let words = |b: &[u8]| {
                    b.chunks(4)
                        .map(|chunk| format!("{} ", hex(chunk)))
                        .collect::<String>()
                };
                format!(
                    "{}... {}",
                    words(&bytes[..16]),
                    words(&bytes[bytes.len() - 8..])
                )
            };
            format!("{{length = {}, bytes = 0x{bytes_text}}}", bytes.len())
        }
        other => element(other, level),
    }
}

/// An element inside an array or dictionary description.
fn element(value: &Value, level: usize) -> String {
    match value {
        Value::String(s) => quoted(s),
        Value::Boolean(b) => u8::from(*b).to_string(),
        Value::Integer(n) => quoted(&n.to_string()),
        Value::Real(n) => quoted(&number_description(*n)),
        Value::Date(_) => quoted(&description(value, 0)),
        _ => description(value, level),
    }
}

/// `NSNumber`'s description of a double: `%g` style with shortest digits.
fn number_description(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e16 {
        return format!("{}", n as i64);
    }
    let exponent = n.abs().log10().floor() as i32;
    if (-4..16).contains(&exponent) {
        return format!("{n}");
    }
    let text = format!("{n:e}");
    let (mantissa, exponent) = text.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    format!(
        "{mantissa}e{}{:02}",
        if exponent < 0 { '-' } else { '+' },
        exponent.abs()
    )
}

/// Quoting in Foundation's old-style property list descriptions.
fn quoted(text: &str) -> String {
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric()) {
        return text.to_owned();
    }
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\U{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Dictionary;

    #[test]
    fn wraps_long_strings_and_nested_values_like_python() {
        let input = Value::Dictionary(Dictionary::from_iter([
            ("NAME", Value::from("VLC")),
            (
                "pkginfo",
                Value::Dictionary(Dictionary::from_iter([
                    ("catalogs", Value::Array(vec!["testing".into()])),
                    ("description", Value::from("VLC is a free and open source cross-platform multimedia player and framework that plays most multimedia files as well as DVD, Audio CD, VCD, and various streaming protocols.")),
                    ("unattended_install", Value::Boolean(true)),
                ])),
            ),
        ]));
        // Captured from Python 3.11: pprint.pformat(value, indent=4).
        assert_eq!(
            pformat(&Py::python(&input), 4),
            "{   'NAME': 'VLC',
    'pkginfo': {   'catalogs': ['testing'],
                   'description': 'VLC is a free and open source '
                                  'cross-platform multimedia player and '
                                  'framework that plays most multimedia files '
                                  'as well as DVD, Audio CD, VCD, and various '
                                  'streaming protocols.',
                   'unattended_install': True}}"
        );
    }

    #[test]
    fn shows_domain_values_as_autopkg_converts_them() {
        let nested = Value::Dictionary(Dictionary::from_iter([(
            "outer",
            Value::Dictionary(Dictionary::from_iter([(
                "inner",
                Value::Array(vec!["1".into(), "two".into()]),
            )])),
        )]));
        let date = plist::Date::from_xml_format("2027-01-02T03:04:05Z").unwrap();
        let prefs = Py::Dict(vec![
            (
                "DATA".into(),
                Py::autopkg_domain(&Value::Data(vec![0, 255, 16, 171])),
            ),
            ("DATE".into(), Py::autopkg_domain(&Value::Date(date))),
            ("NESTED".into(), Py::autopkg_domain(&nested)),
            ("REAL".into(), Py::autopkg_domain(&Value::Real(2.75))),
            ("TRUE".into(), Py::autopkg_domain(&Value::Boolean(true))),
        ]);
        // Captured from AutoPkg 3.0.0's `autopkg info`.
        assert_eq!(
            pformat(&prefs, 1),
            "{'DATA': {length = 4, bytes = 0x00ff10ab},
 'DATE': 2027-01-02 03:04:05 +0000,
 'NESTED': {'outer': {'inner': (
    1,
    two
)}},
 'REAL': 2,
 'TRUE': True}"
        );
    }

    #[test]
    fn foundation_descriptions_match_pyobjc() {
        let array = Value::Array(vec![
            "two".into(),
            "/Users/x".into(),
            "".into(),
            "q\"uote".into(),
            "üñí".into(),
            Value::from(1),
            Value::Real(2.5),
            Value::Boolean(true),
            Value::from(-5),
            Value::Real(1e20),
            Value::Real(1e-7),
            Value::Array(vec![Value::from(1), "x".into()]),
            Value::Dictionary(Dictionary::from_iter([("k", "v")])),
        ]);
        // Captured from PyObjC's repr() of the same NSArray.
        assert_eq!(
            description(&array, 0),
            "(
    two,
    \"/Users/x\",
    \"\",
    \"q\\\"uote\",
    \"\\U00fc\\U00f1\\U00ed\",
    1,
    \"2.5\",
    1,
    \"-5\",
    \"1e+20\",
    \"1e-07\",
        (
        1,
        x
    ),
        {
        k = v;
    }
)"
        );
        assert_eq!(
            description(&Value::Data((0..25).collect()), 0),
            "{length = 25, bytes = 0x00010203 04050607 08090a0b 0c0d0e0f ... 11121314 15161718 }"
        );
        assert_eq!(description(&Value::Array(vec![]), 0), "(\n)");
        assert_eq!(
            description(&Value::Dictionary(Dictionary::new()), 0),
            "{\n}"
        );
    }

    #[test]
    fn splits_lines_and_words_like_python() {
        assert_eq!(split_lines("a\nb\r\nc"), ["a\n", "b\r\n", "c"]);
        assert_eq!(words("one  two three "), ["one  ", "two ", "three "]);
        assert_eq!(words("  lead"), ["  ", "lead"]);
    }
}
