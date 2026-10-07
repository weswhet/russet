//! The code requirement language that `codesign -R` and
//! `CodeSignatureVerifier`'s `requirement` use.
//!
//! Supported: `identifier`, `anchor apple generic`, `certificate`
//! (`leaf`, `root`, or a position) with `[subject.*]` or `[field.OID]`,
//! `cdhash`, `always`, `never`, `and`, `or`, `not`, parentheses, and
//! comments. Anything else is an error, so a requirement is never treated
//! as satisfied when the verifier doesn't understand it.

use crate::trust::{Cert, Chain};
use der::asn1::ObjectIdentifier;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Hash(String),
    Open,
    Close,
    BracketOpen,
    BracketClose,
    Equals,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            _ if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'*') => {
                let end = (i + 2..chars.len().saturating_sub(1))
                    .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
                    .ok_or("Unterminated comment in requirement")?;
                i = end + 2;
            }
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '[' => {
                tokens.push(Token::BracketOpen);
                i += 1;
            }
            ']' => {
                tokens.push(Token::BracketClose);
                i += 1;
            }
            '=' => {
                tokens.push(Token::Equals);
                i += 1;
            }
            '"' => {
                let mut value = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("Unterminated string in requirement".into()),
                        Some('"') => break,
                        Some('\\') => {
                            value.push(*chars.get(i + 1).ok_or("Bad escape in requirement")?);
                            i += 2;
                        }
                        Some(c) => {
                            value.push(*c);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Quoted(value));
                i += 1;
            }
            'H' if chars.get(i + 1) == Some(&'"') => {
                let end = (i + 2..chars.len())
                    .find(|&j| chars[j] == '"')
                    .ok_or("Unterminated hash in requirement")?;
                tokens.push(Token::Hash(chars[i + 2..end].iter().collect()));
                i = end + 1;
            }
            _ if c.is_alphanumeric() || "._-*:".contains(c) => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || "._-*:".contains(chars[i]))
                {
                    i += 1;
                }
                tokens.push(Token::Word(chars[start..i].iter().collect()));
            }
            '<' | '>' | '~' => {
                return Err(format!(
                    "Requirement operator '{c}' isn't supported by the native verifier"
                ))
            }
            _ => return Err(format!("Unexpected '{c}' in requirement")),
        }
    }
    Ok(tokens)
}

/// Which certificate a `certificate` clause names.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    /// Counting from the leaf (0).
    FromLeaf(usize),
    /// Counting back from the root (1 is the root).
    FromRoot(usize),
}

#[derive(Clone, Debug, PartialEq)]
enum Field {
    Subject(ObjectIdentifier),
    Extension(ObjectIdentifier),
}

#[derive(Clone, Debug, PartialEq)]
enum Match {
    Exists,
    Equals(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Always(bool),
    Identifier(String),
    AnchorAppleGeneric,
    Certificate(Slot, Field, Match),
    CdHash(Vec<u8>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

fn unsupported(what: &str) -> String {
    format!("Requirement clause '{what}' isn't supported by the native verifier")
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).cloned();
        self.at += 1;
        token
    }

    fn word(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token::Word(w)) => Ok(w),
            other => Err(format!(
                "Expected a keyword in requirement, found {other:?}"
            )),
        }
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w == word)
    }

    fn value(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Token::Word(w)) | Some(Token::Quoted(w)) => Ok(w),
            other => Err(format!("Expected a value in requirement, found {other:?}")),
        }
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.keyword("or") {
            self.at += 1;
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while self.keyword("and") {
            self.at += 1;
            left = Expr::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.keyword("not") || matches!(self.peek(), Some(Token::Word(w)) if w == "!") {
            self.at += 1;
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        if self.peek() == Some(&Token::Open) {
            self.at += 1;
            let inner = self.or()?;
            if self.next() != Some(Token::Close) {
                return Err("Missing ')' in requirement".into());
            }
            return Ok(inner);
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let word = self.word()?;
        match word.as_str() {
            "always" | "true" => Ok(Expr::Always(true)),
            "never" | "false" => Ok(Expr::Always(false)),
            "identifier" => {
                if self.peek() == Some(&Token::Equals) {
                    self.at += 1;
                }
                Ok(Expr::Identifier(self.value()?))
            }
            "anchor" => {
                let kind = self.word()?;
                if kind == "apple" && self.keyword("generic") {
                    self.at += 1;
                    Ok(Expr::AnchorAppleGeneric)
                } else {
                    Err(unsupported(&format!("anchor {kind}")))
                }
            }
            "certificate" | "cert" => {
                let slot = match self.value()?.as_str() {
                    "leaf" => Slot::FromLeaf(0),
                    "root" | "anchor" => Slot::FromRoot(1),
                    n => match n.parse::<i64>() {
                        Ok(n) if n >= 0 => Slot::FromLeaf(n as usize),
                        Ok(n) => Slot::FromRoot(n.unsigned_abs() as usize),
                        Err(_) => {
                            return Err(format!("Bad certificate position '{n}' in requirement"))
                        }
                    },
                };
                if self.next() != Some(Token::BracketOpen) {
                    return Err(unsupported("certificate without a [field]"));
                }
                let name = self.value()?;
                if self.next() != Some(Token::BracketClose) {
                    return Err("Missing ']' in requirement".into());
                }
                let field = if let Some(oid) = name.strip_prefix("field.") {
                    Field::Extension(
                        oid.parse()
                            .map_err(|_| format!("Bad OID '{oid}' in requirement"))?,
                    )
                } else if let Some(attribute) = name.strip_prefix("subject.") {
                    Field::Subject(match attribute {
                        "CN" => ObjectIdentifier::new_unwrap("2.5.4.3"),
                        "C" => ObjectIdentifier::new_unwrap("2.5.4.6"),
                        "L" => ObjectIdentifier::new_unwrap("2.5.4.7"),
                        "ST" => ObjectIdentifier::new_unwrap("2.5.4.8"),
                        "O" => ObjectIdentifier::new_unwrap("2.5.4.10"),
                        "OU" => ObjectIdentifier::new_unwrap("2.5.4.11"),
                        "email" | "emailAddress" => {
                            ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1")
                        }
                        other => return Err(unsupported(&format!("subject.{other}"))),
                    })
                } else {
                    return Err(unsupported(&format!("certificate field {name}")));
                };
                let matcher = match self.peek() {
                    Some(Token::Equals) => {
                        self.at += 1;
                        Match::Equals(self.value()?)
                    }
                    Some(Token::Word(w)) if w == "exists" => {
                        self.at += 1;
                        Match::Exists
                    }
                    Some(Token::Word(w)) if w == "absent" => return Err(unsupported("absent")),
                    _ => Match::Exists,
                };
                Ok(Expr::Certificate(slot, field, matcher))
            }
            "cdhash" => {
                if self.peek() == Some(&Token::Equals) {
                    self.at += 1;
                }
                let hex = match self.next() {
                    Some(Token::Hash(h)) | Some(Token::Quoted(h)) | Some(Token::Word(h)) => h,
                    other => {
                        return Err(format!("Expected a hash in requirement, found {other:?}"))
                    }
                };
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("zz"), 16))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "Bad cdhash in requirement".to_string())?;
                Ok(Expr::CdHash(bytes))
            }
            other => Err(unsupported(other)),
        }
    }
}

/// A parsed requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct Requirement(Expr);

impl Requirement {
    /// Parses requirement text. A leading `=`, as `codesign -R=` accepts, and
    /// a `designated =>` prefix are allowed.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let text = text.strip_prefix('=').unwrap_or(text).trim();
        let text = text
            .strip_prefix("designated")
            .map(|t| t.trim_start().trim_start_matches("=>"))
            .unwrap_or(text);
        let mut parser = Parser {
            tokens: tokenize(text)?,
            at: 0,
        };
        let expr = parser.or()?;
        if parser.at != parser.tokens.len() {
            return Err(format!("Unexpected {:?} in requirement", parser.peek()));
        }
        Ok(Self(expr))
    }

    /// Evaluates the requirement for code with this identifier, signing
    /// chain (`None` for ad-hoc signatures), and code directory hashes.
    pub fn evaluate(&self, context: &Context) -> bool {
        evaluate(&self.0, context)
    }
}

/// What a requirement is evaluated against.
pub struct Context<'a> {
    pub identifier: &'a str,
    pub chain: Option<&'a Chain>,
    pub cdhashes: &'a [Vec<u8>],
}

fn certificate(chain: &Chain, slot: Slot) -> Option<&Cert> {
    let certs = &chain.certificates;
    match slot {
        Slot::FromLeaf(n) => certs.get(n),
        Slot::FromRoot(n) => certs.len().checked_sub(n).and_then(|i| certs.get(i)),
    }
}

fn matches(value: &str, pattern: &str) -> bool {
    match (pattern.strip_prefix('*'), pattern.strip_suffix('*')) {
        (Some(rest), _) if rest.ends_with('*') => value.contains(&rest[..rest.len() - 1]),
        (Some(suffix), _) => value.ends_with(suffix),
        (_, Some(prefix)) => value.starts_with(prefix),
        _ => value == pattern,
    }
}

fn evaluate(expr: &Expr, context: &Context) -> bool {
    match expr {
        Expr::Always(value) => *value,
        Expr::Identifier(id) => context.identifier == id,
        // The chain was validated to one of Apple's roots.
        Expr::AnchorAppleGeneric => context.chain.is_some(),
        Expr::Certificate(slot, field, matcher) => {
            let Some(cert) = context.chain.and_then(|c| certificate(c, *slot)) else {
                return false;
            };
            match field {
                Field::Extension(oid) => match matcher {
                    Match::Exists => cert.has_extension(oid),
                    Match::Equals(_) => false,
                },
                Field::Subject(oid) => {
                    let values = cert.subject_values(oid);
                    match matcher {
                        Match::Exists => !values.is_empty(),
                        Match::Equals(pattern) => values.iter().any(|v| matches(v, pattern)),
                    }
                }
            }
        }
        Expr::CdHash(hash) => context.cdhashes.iter().any(|h| h == hash),
        Expr::Not(inner) => !evaluate(inner, context),
        Expr::And(a, b) => evaluate(a, context) && evaluate(b, context),
        Expr::Or(a, b) => evaluate(a, context) || evaluate(b, context),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_core_recipe_requirements() {
        let chrome = r#"(identifier "com.google.Chrome" or identifier "com.google.Chrome.beta") and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = EQHXZ8M8AV"#;
        let parsed = Requirement::parse(chrome).unwrap();
        let context = Context {
            identifier: "com.google.Chrome",
            chain: None,
            cdhashes: &[],
        };
        // Without a chain, nothing anchored at Apple is satisfied.
        assert!(!parsed.evaluate(&context));
        assert!(!Requirement::parse(r#"identifier "a" or never"#)
            .unwrap()
            .evaluate(&context));
        assert!(
            Requirement::parse(r#"identifier "com.google.Chrome" and always"#)
                .unwrap()
                .evaluate(&context)
        );
        assert!(Requirement::parse("=identifier com.google.Chrome")
            .unwrap()
            .evaluate(&context));
    }

    #[test]
    fn rejects_unsupported_clauses() {
        for text in [
            "anchor apple",
            "notarized",
            "info [CFBundleVersion] = 1",
            "entitlement [a] exists",
            "certificate leaf[subject.OU] < 5",
            "certificate leaf trusted",
        ] {
            assert!(Requirement::parse(text).is_err(), "{text}");
        }
    }
}
