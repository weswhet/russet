//! Python-compatible representations used in substitution and processor output.
use crate::Value;

pub fn python_quote(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for ch in value.chars() {
        match ch {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            ch if ch == quote => {
                result.push('\\');
                result.push(ch);
            }
            ch if ch.is_control() => result.push_str(&format!("\\x{:02x}", ch as u32)),
            _ => result.push(ch),
        }
    }
    result.push(quote);
    result
}
pub fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::String(s) => python_quote(s),
        Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
        Value::Integer(n) => n.to_string(),
        Value::Real(n) => {
            if n.is_nan() {
                return "nan".into();
            }
            let text = format!("{n:?}");
            if let Some((mantissa, exponent)) = text.split_once('e') {
                if let Ok(exponent) = exponent.parse::<i32>() {
                    return format!("{mantissa}e{exponent:+03}");
                }
            }
            text
        }
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Dictionary(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(k, v)| format!("{}: {}", python_quote(k), python_repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Data(bytes) => {
            let quote = if bytes.contains(&b'\'') && !bytes.contains(&b'"') {
                '"'
            } else {
                '\''
            };
            let mut result = format!("b{quote}");
            for byte in bytes {
                match byte {
                    b'\n' => result.push_str("\\n"),
                    b'\r' => result.push_str("\\r"),
                    b'\t' => result.push_str("\\t"),
                    b'\\' => result.push_str("\\\\"),
                    b if *b == quote as u8 => {
                        result.push('\\');
                        result.push(quote);
                    }
                    32..=126 => result.push(*byte as char),
                    _ => result.push_str(&format!("\\x{byte:02x}")),
                }
            }
            result.push(quote);
            result
        }
        // Dates inside containers require Python's datetime repr; standalone
        // substitutions use its str() shape above. Retain an explicit shape.
        Value::Date(date) => {
            let text = date.to_xml_format();
            let (whole, fraction) = text
                .trim_end_matches('Z')
                .split_once('.')
                .unwrap_or((text.trim_end_matches('Z'), ""));
            let mut parts: Vec<_> = whole
                .split(['-', 'T', ':'])
                .filter_map(|p| p.parse::<u32>().ok())
                .map(|n| n.to_string())
                .collect();
            let microseconds = format!("{fraction:0<6}")
                .chars()
                .take(6)
                .collect::<String>()
                .parse::<u32>()
                .unwrap_or(0);
            if microseconds > 0 {
                parts.push(microseconds.to_string());
            } else if parts.len() == 6 && parts.last().is_some_and(|p| p == "0") {
                parts.pop();
            }
            format!("datetime.datetime({})", parts.join(", "))
        }
        _ => format!("{value:?}"),
    }
}
pub fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Date(date) => date
            .to_xml_format()
            .replace('T', " ")
            .trim_end_matches('Z')
            .to_owned(),
        other => python_repr(other),
    }
}
