//! Deliberately bounded NSPredicate-style portable evaluator.
use super::Result;
use plist::{Dictionary, Value};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
#[derive(Debug, Clone)]
enum Number {
    Integer(i128),
    Real(f64),
}
impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        fn integer_real(integer: i128, real: f64) -> Option<std::cmp::Ordering> {
            if !real.is_finite() {
                return (integer as f64).partial_cmp(&real);
            }
            if real >= i128::MAX as f64 {
                return Some(std::cmp::Ordering::Less);
            }
            if real < i128::MIN as f64 {
                return Some(std::cmp::Ordering::Greater);
            }
            let truncated = real as i128;
            let order = integer.cmp(&truncated);
            if order != std::cmp::Ordering::Equal {
                Some(order)
            } else {
                0.0_f64.partial_cmp(&real.fract())
            }
        }
        match (self, other) {
            (Self::Integer(a), Self::Integer(b)) => Some(a.cmp(b)),
            (Self::Real(a), Self::Real(b)) => a.partial_cmp(b),
            (Self::Integer(a), Self::Real(b)) => integer_real(*a, *b),
            (Self::Real(a), Self::Integer(b)) => {
                integer_real(*b, *a).map(std::cmp::Ordering::reverse)
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    String(String),
    Number(Number),
    Symbol(String),
}
#[derive(Debug, Clone, PartialEq)]
enum Item {
    Null,
    Number(Number),
    String(String),
    Array(Vec<Item>),
}
fn value(v: &Value) -> Result<Item> {
    Ok(match v {
        Value::Null => Item::Null,
        Value::String(s) => Item::String(s.clone()),
        Value::Boolean(b) => Item::Number(Number::Integer(i128::from(*b))),
        Value::Integer(i) => Item::Number(Number::Integer(
            i.as_signed()
                .map(i128::from)
                .or_else(|| i.as_unsigned().map(i128::from))
                .ok_or("Invalid integer")?,
        )),
        Value::Real(n) => Item::Number(Number::Real(*n)),
        Value::Array(a) => Item::Array(a.iter().map(value).collect::<Result<_>>()?),
        _ => return Err("Unsupported predicate value type".into()),
    })
}
fn lex(s: &str) -> Result<Vec<Token>> {
    let mut chars = s.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '\'' || c == '"' {
            let mut text = String::new();
            let mut closed = false;
            while let Some(n) = chars.next() {
                if n == c {
                    closed = true;
                    break;
                }
                if n == '\\' {
                    let n = chars.next().ok_or("Unterminated string escape")?;
                    text.push(match n {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        n if n == c || n == '\\' => n,
                        _ => return Err("Unsupported string escape".into()),
                    });
                } else {
                    text.push(n);
                }
            }
            if !closed {
                return Err("Unterminated string literal".into());
            }
            out.push(Token::String(text));
        } else if c.is_ascii_digit() || c == '-' || c == '+' {
            let mut n = c.to_string();
            while chars.peek().is_some_and(|n| {
                n.is_ascii_digit() || *n == '.' || *n == 'e' || *n == 'E' || *n == '+' || *n == '-'
            }) {
                n.push(chars.next().unwrap());
            }
            let number = if !n.contains(['.', 'e', 'E']) {
                Number::Integer(n.parse().map_err(|_| "Integer literal out of range")?)
            } else {
                let n: f64 = n.parse().map_err(|_| "Invalid numeric literal")?;
                if !n.is_finite() {
                    return Err("Nonfinite numeric literal".into());
                }
                Number::Real(n)
            };
            out.push(Token::Number(number));
        } else if c.is_alphabetic() || c == '_' {
            let mut word = c.to_string();
            while chars
                .peek()
                .is_some_and(|n| n.is_alphanumeric() || *n == '_' || *n == '.')
            {
                word.push(chars.next().unwrap());
            }
            out.push(Token::Word(word));
        } else if "(){}[],=<>!&|".contains(c) {
            let mut symbol = c.to_string();
            if chars.peek().is_some_and(|n| {
                (*n == '=' && "=<>!".contains(c))
                    || (*n == c && "&|".contains(c))
                    || (c == '<' && *n == '>')
            }) {
                symbol.push(chars.next().unwrap());
            }
            out.push(Token::Symbol(symbol));
        } else {
            return Err(format!("Unsupported predicate character: {c}"));
        }
    }
    Ok(out)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    env: &'a Dictionary,
}
impl Parser<'_> {
    fn take(&mut self, text: &str) -> bool {
        if matches!(self.tokens.get(self.pos), Some(Token::Word(s) | Token::Symbol(s)) if s.eq_ignore_ascii_case(text))
        {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expression(&mut self) -> Result<bool> {
        let mut result = self.and()?;
        while self.take("OR") || self.take("||") {
            let rhs = self.and()?;
            result = result || rhs;
        }
        Ok(result)
    }
    fn and(&mut self) -> Result<bool> {
        let mut result = self.unary()?;
        while self.take("AND") || self.take("&&") {
            let rhs = self.unary()?;
            result = result && rhs;
        }
        Ok(result)
    }
    fn unary(&mut self) -> Result<bool> {
        if self.take("NOT") || self.take("!") {
            return Ok(!self.unary()?);
        }
        if self.take("(") {
            let result = self.expression()?;
            if !self.take(")") {
                return Err("Expected closing parenthesis".into());
            }
            return Ok(result);
        }
        if self.take("TRUEPREDICATE") {
            return Ok(true);
        }
        if self.take("FALSEPREDICATE") {
            return Ok(false);
        }
        let lhs = self.item()?;
        let op = match self.tokens.get(self.pos) {
            Some(Token::Word(s) | Token::Symbol(s)) => s.to_uppercase(),
            _ => return Err("Expected comparison operator".into()),
        };
        self.pos += 1;
        if ![
            "=",
            "==",
            "!=",
            "<>",
            "<",
            "<=",
            ">",
            ">=",
            "IN",
            "CONTAINS",
            "BEGINSWITH",
            "ENDSWITH",
            "LIKE",
            "MATCHES",
        ]
        .contains(&op.as_str())
        {
            return Err(format!("Unsupported predicate operator: {op}"));
        }
        let mut modifiers = String::new();
        if self.take("[") {
            if let Some(Token::Word(s)) = self.tokens.get(self.pos) {
                modifiers = s.to_lowercase();
                self.pos += 1;
            }
            if modifiers.is_empty()
                || !modifiers.chars().all(|c| c == 'c' || c == 'd')
                || !self.take("]")
            {
                return Err("Unsupported comparison modifier".into());
            }
        }
        let rhs = self.item()?;
        compare(lhs, &op, rhs, &modifiers)
    }
    fn item(&mut self) -> Result<Item> {
        if self.take("{") {
            let mut items = Vec::new();
            if self.take("}") {
                return Ok(Item::Array(items));
            }
            loop {
                items.push(self.item()?);
                if self.take("}") {
                    break;
                }
                if !self.take(",") {
                    return Err("Expected comma or closing brace".into());
                }
            }
            return Ok(Item::Array(items));
        }
        let token = self
            .tokens
            .get(self.pos)
            .cloned()
            .ok_or("Expected predicate operand")?;
        self.pos += 1;
        match token {
            Token::String(s) => Ok(Item::String(s)),
            Token::Number(n) => Ok(Item::Number(n)),
            Token::Word(s) => {
                match s.to_uppercase().as_str() {
                    "TRUE" | "YES" => return Ok(Item::Number(Number::Integer(1))),
                    "FALSE" | "NO" => return Ok(Item::Number(Number::Integer(0))),
                    "NIL" | "NULL" => return Ok(Item::Null),
                    _ => (),
                }
                let mut parts = s.split('.');
                let first = parts.next().unwrap();
                let mut current = if first.eq_ignore_ascii_case("SELF") {
                    parts.next().and_then(|p| self.env.get(p))
                } else {
                    self.env.get(first)
                };
                for part in parts {
                    if part.is_empty() {
                        return Err("Invalid environment key path".into());
                    }
                    current = current
                        .and_then(Value::as_dictionary)
                        .and_then(|d| d.get(part));
                }
                current.map(value).unwrap_or(Ok(Item::Null))
            }
            _ => Err("Expected literal or environment key path".into()),
        }
    }
}
fn normalized(s: String, modifiers: &str) -> String {
    let s = if modifiers.contains('d') {
        s.nfd().filter(|c| !is_combining_mark(*c)).collect()
    } else {
        s
    };
    if modifiers.contains('c') {
        s.to_lowercase()
    } else {
        s
    }
}
fn normalize(item: Item, modifiers: &str) -> Item {
    match item {
        Item::String(s) => Item::String(normalized(s, modifiers)),
        Item::Array(a) => Item::Array(a.into_iter().map(|v| normalize(v, modifiers)).collect()),
        other => other,
    }
}
fn compare(lhs: Item, op: &str, rhs: Item, modifiers: &str) -> Result<bool> {
    // Foundation collection membership uses element equality, not string modifiers.
    let modifiers = if matches!(lhs, Item::Array(_)) || matches!(rhs, Item::Array(_)) {
        ""
    } else {
        modifiers
    };
    let lhs = normalize(lhs, modifiers);
    let rhs = normalize(rhs, modifiers);
    if op == "=" || op == "==" {
        return Ok(lhs == rhs);
    }
    if op == "!=" || op == "<>" {
        return Ok(lhs != rhs);
    }
    if op == "IN" {
        return match rhs {
            Item::Array(a) => Ok(a.contains(&lhs)),
            Item::String(s) => match lhs {
                Item::String(sub) => Ok(s.contains(&sub)),
                _ => Err("IN requires compatible operands".into()),
            },
            _ => Err("IN requires an array or string".into()),
        };
    }
    if op == "CONTAINS" {
        if let Item::Array(a) = &lhs {
            return Ok(a.contains(&rhs));
        }
    }
    match (&lhs, &rhs) {
        (Item::String(a), Item::String(b)) => match op {
            "CONTAINS" => Ok(a.contains(b)),
            "BEGINSWITH" => Ok(a.starts_with(b)),
            "ENDSWITH" => Ok(a.ends_with(b)),
            "LIKE" => {
                let pattern: String = b
                    .chars()
                    .map(|c| match c {
                        '*' => ".*".to_string(),
                        '?' => ".".to_string(),
                        c => regex::escape(&c.to_string()),
                    })
                    .collect();
                Ok(regex::Regex::new(&format!("(?s)\\A(?:{pattern})\\z"))
                    .map_err(|e| e.to_string())?
                    .is_match(a))
            }
            "MATCHES" => Ok(regex::Regex::new(&format!("\\A(?:{b})\\z"))
                .map_err(|e| format!("Unsupported or invalid predicate regex: {e}"))?
                .is_match(a)),
            _ => ordered(a.partial_cmp(b), op),
        },
        (Item::Number(a), Item::Number(b)) => ordered(a.partial_cmp(b), op),
        _ => Err(format!("Incompatible operands for {op}")),
    }
}
fn ordered(order: Option<std::cmp::Ordering>, op: &str) -> Result<bool> {
    use std::cmp::Ordering::*;
    match op {
        "<" => Ok(order == Some(Less)),
        "<=" => Ok(matches!(order, Some(Less | Equal))),
        ">" => Ok(order == Some(Greater)),
        ">=" => Ok(matches!(order, Some(Greater | Equal))),
        _ => Err(format!("Unsupported operand types for {op}")),
    }
}
pub(super) fn evaluate(source: &str, env: &Dictionary) -> Result<bool> {
    let mut parser = Parser {
        tokens: lex(source)?,
        pos: 0,
        env,
    };
    let result = parser.expression()?;
    if parser.pos != parser.tokens.len() {
        return Err("Unsupported trailing predicate syntax".into());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_evaluator_matches_captured_native_oracle() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/predicate-fixtures.json"
        ))
        .unwrap();
        let reference: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../compatibility/predicate-reference-results.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let env: Dictionary = serde_json::from_value(case["environment"].clone()).unwrap();
            let result = evaluate(env["predicate"].as_string().unwrap(), &env);
            let expected = &reference["cases"][name];
            if expected["status"] == 0 {
                assert_eq!(
                    result.unwrap(),
                    expected["environment"]["stop_processing_recipe"]
                        .as_bool()
                        .unwrap(),
                    "{name}"
                );
            } else {
                assert!(result.is_err(), "{name}");
            }
        }
    }
    #[test]
    fn boolean_precedence_membership_and_modifiers() {
        let mut env = Dictionary::new();
        env.insert("name".into(), "CAFÉ".into());
        env.insert("count".into(), 4.into());
        assert!(evaluate(
            "name ==[cd] 'cafe' AND (count >= 3 OR FALSEPREDICATE)",
            &env
        )
        .unwrap());
        assert!(evaluate("count IN {1, 4, 9} AND NOT name BEGINSWITH 'x'", &env).unwrap());
        assert!(evaluate("name LIKE[c] 'ca*'", &env).unwrap());
        assert!(evaluate("missing == NIL", &env).unwrap());
    }
    #[test]
    fn rejects_unsupported_even_in_unused_branch() {
        let e = Dictionary::new();
        for s in [
            "TRUEPREDICATE OR FUNCTION(x, 'bad')",
            "1 == 1 garbage",
            "'x' MATCHES '(?=x)'",
            "name ==[z] 'a'",
            "(TRUEPREDICATE",
            "name == 'unterminated",
        ] {
            assert!(evaluate(s, &e).is_err(), "{s}");
        }
    }
    #[test]
    fn keypaths_and_arrays() {
        let mut e = Dictionary::new();
        let mut nested = Dictionary::new();
        nested.insert("enabled".into(), true.into());
        e.insert("nested".into(), nested.into());
        e.insert("names".into(), Value::Array(vec!["Résumé".into()]));
        assert!(!evaluate("nested.enabled == YES AND names CONTAINS[cd] 'resume'", &e).unwrap());
    }
}
