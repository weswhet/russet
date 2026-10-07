//! Python string-regex ASCII semantics without rewriting the input alphabet.
//! ASCII classes and literal folding are translated in each pattern scope;
//! the VM's Python ASCII backreferences preserve exact non-ASCII identity.
#[derive(Debug)]
pub(super) struct Prepared {
    pub pattern: String,
    pub text: String,
}
impl Prepared {
    pub fn original<'a>(&self, content: &'a str, matched: fancy_regex::Match<'_>) -> &'a str {
        &content[matched.start()..matched.end()]
    }
}

fn hex_escape(chars: &[char], start: usize) -> Option<(usize, char)> {
    if chars.get(start) != Some(&'\\') {
        return None;
    }
    let count = match chars.get(start + 1)? {
        'x' => 2,
        'u' => 4,
        'U' => 8,
        _ => return None,
    };
    let digits = chars.get(start + 2..start + 2 + count)?;
    if !digits.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let scalar = u32::from_str_radix(&digits.iter().collect::<String>(), 16).ok()?;
    Some((start + count + 2, char::from_u32(scalar)?))
}
fn flags(chars: &[char], start: usize) -> Option<(usize, Vec<char>, char)> {
    if chars.get(start..start + 2)? != ['(', '?'] {
        return None;
    }
    let mut end = start + 2;
    while chars.get(end).is_some_and(|c| "aiLmsux-".contains(*c)) {
        end += 1;
    }
    let delimiter = *chars.get(end)?;
    if end == start + 2 || !matches!(delimiter, ':' | ')') {
        return None;
    }
    Some((end + 1, chars[start + 2..end].to_vec(), delimiter))
}

pub(super) fn prepare(
    pattern: &str,
    content: &str,
    mut ascii: bool,
    ignore_case: bool,
) -> Result<Prepared, String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut leading = 0;
    while let Some((end, fs, ')')) = flags(&chars, leading) {
        if fs.contains(&'a') {
            ascii = true;
        }
        if fs.contains(&'u') && ascii {
            return Err("ASCII and UNICODE flags are incompatible".into());
        }
        leading = end;
    }
    let base_ascii = ascii;
    let text = content.to_owned();
    let mut output = String::new();
    let mut modes = Vec::new();
    let mut casei = ignore_case;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if let Some((end, fs, delimiter)) = flags(&chars, index) {
            let mut neg = false;
            let mut next_casei = casei;
            let mut next_ascii = ascii;
            let mut retained = String::new();
            for f in fs {
                match f {
                    '-' => {
                        neg = true;
                        retained.push(f);
                    }
                    'a' => {
                        if neg {
                            return Err("ASCII flag cannot be disabled".into());
                        }
                        next_ascii = true;
                    }
                    'u' => {
                        if neg {
                            return Err("UNICODE flag cannot be disabled".into());
                        }
                        next_ascii = false;
                    }
                    'L' => return Err("cannot use LOCALE flag with a str pattern".into()),
                    'i' => {
                        next_casei = !neg;
                        retained.push(f);
                    }
                    _ => retained.push(f),
                }
            }
            if delimiter == ':' {
                modes.push((casei, ascii));
            }
            casei = next_casei;
            ascii = next_ascii;
            if retained.is_empty() {
                output.push_str(if delimiter == ':' { "(?:" } else { "(?:)" });
            } else {
                output.push_str("(?");
                output.push_str(&retained);
                output.push(delimiter);
            }
            index = end;
            continue;
        }
        if c == '(' {
            // Preserve group names, conditional identifiers, and comments.
            let prefix: String = chars[index..].iter().take(5).collect();
            let terminator = if prefix.starts_with("(?P<") {
                Some('>')
            } else if prefix.starts_with("(?P=")
                || prefix.starts_with("(?#")
                || prefix.starts_with("(?(")
            {
                Some(')')
            } else {
                None
            };
            if let Some(terminator) = terminator {
                let end = chars[index + 3..]
                    .iter()
                    .position(|c| *c == terminator)
                    .map(|n| n + index + 4)
                    .ok_or("Unclosed regex group header")?;
                if prefix.starts_with("(?P<") || prefix.starts_with("(?(") {
                    modes.push((casei, ascii));
                }
                if prefix.starts_with("(?P=") && ascii && casei {
                    output.push_str("(?a:");
                }
                output.extend(chars[index..end].iter());
                if prefix.starts_with("(?P=") && ascii && casei {
                    output.push(')');
                }
                index = end;
                continue;
            }
            modes.push((casei, ascii));
        } else if c == ')' {
            (casei, ascii) = modes.pop().unwrap_or((ignore_case, base_ascii));
        }
        if c == '[' {
            let mut end = index + 1;
            if chars.get(end) == Some(&'^') {
                end += 1;
            }
            if chars.get(end) == Some(&']') {
                end += 1;
            }
            while end < chars.len() {
                if chars[end] == '\\' {
                    end += 2;
                } else if chars[end] == ']' {
                    break;
                } else {
                    end += 1;
                }
            }
            if end >= chars.len() {
                return Err("Unclosed regex character class".into());
            }
            let raw: String = chars[index..=end].iter().collect();
            let mut class = raw.clone();
            if !ascii && casei {
                let positive = if let Some(body) = raw.strip_prefix("[^") {
                    format!("[{body}")
                } else {
                    raw.clone()
                };
                if regex::RegexBuilder::new(&format!("\\A{positive}\\z"))
                    .case_insensitive(true)
                    .build()
                    .is_ok_and(|re| re.is_match("i") || re.is_match("İ") || re.is_match("ı"))
                {
                    class.pop();
                    class.push_str("iIİı]");
                }
            }
            if ascii {
                class = ascii_class(&class, casei)?;
            }
            output.push_str(&class);
            index = end + 1;
            continue;
        }
        if c == '\\' {
            if let Some((end, value)) = hex_escape(&chars, index) {
                if ascii && casei {
                    let literal = if value.is_ascii_alphabetic() {
                        format!(
                            "[{}{}]",
                            value.to_ascii_lowercase(),
                            value.to_ascii_uppercase()
                        )
                    } else {
                        regex::escape(&value.to_string())
                    };
                    output.push_str(&format!("(?-i:{literal})"));
                    index = end;
                    continue;
                }
                if !ascii && casei && matches!(value, 'i' | 'I' | 'İ' | 'ı') {
                    output.push_str("[iIİı]");
                    index = end;
                    continue;
                }
                output.extend(chars[index..end].iter());
                index = end;
                continue;
            }
            let next = *chars.get(index + 1).ok_or("Trailing regex escape")?;
            if ascii && matches!(next, 'w' | 'W' | 'd' | 'D' | 's' | 'S' | 'b' | 'B') {
                output.push_str(ascii_escape(next));
            } else if ascii && casei && next.is_ascii_digit() && next != '0' {
                let mut end = index + 2;
                while chars.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                output.push_str("(?a:");
                output.extend(chars[index..end].iter());
                output.push(')');
                index = end;
                continue;
            } else if next == 'B' {
                output.push_str("(?s:(?:(?<=.)|(?=.)))\\B");
            } else {
                output.push(c);
                output.push(next);
            }
            index += 2;
            continue;
        }
        if !ascii && casei && matches!(c, 'i' | 'I' | 'İ' | 'ı') {
            output.push_str("[iIİı]");
        } else if ascii && casei && (c.is_ascii_alphabetic() || !c.is_ascii()) {
            if c.is_ascii() {
                output.push_str(&format!(
                    "(?-i:[{}{}])",
                    c.to_ascii_lowercase(),
                    c.to_ascii_uppercase()
                ));
            } else {
                output.push_str(&format!("(?-i:{})", regex::escape(&c.to_string())));
            }
        } else {
            output.push(c);
        }
        index += 1;
    }
    Ok(Prepared {
        pattern: output,
        text,
    })
}

fn ascii_escape(c: char) -> &'static str {
    match c {
        'w' => "(?-i:[A-Za-z0-9_])", 'W' => "(?-i:[^A-Za-z0-9_])",
        'd' => "[0-9]", 'D' => "[^0-9]", 's' => "[ \\t\\n\\r\\x0b\\x0c]", 'S' => "[^ \\t\\n\\r\\x0b\\x0c]",
        'b' => "(?-i:(?:(?<=[A-Za-z0-9_])(?![A-Za-z0-9_])|(?<![A-Za-z0-9_])(?=[A-Za-z0-9_])))",
        'B' => "(?s:(?:(?<=.)|(?=.)))(?-i:(?:(?<=[A-Za-z0-9_])(?=[A-Za-z0-9_])|(?<![A-Za-z0-9_])(?![A-Za-z0-9_])))",
        _ => unreachable!(),
    }
}
fn ascii_class(source: &str, casei: bool) -> Result<String, String> {
    // Expand shorthand classes inside the class before disabling Unicode folding.
    let mut body = String::new();
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            body.push(c);
            continue;
        }
        let next = chars.next().ok_or("Trailing class escape")?;
        match next {
            'w' => body.push_str("A-Za-z0-9_"),
            'd' => body.push_str("0-9"),
            's' => body.push_str(" \\t\\n\\r\\x0b\\x0c"),
            'W' => body.push_str("[^A-Za-z0-9_]"),
            'D' => body.push_str("[^0-9]"),
            'S' => body.push_str("[^ \\t\\n\\r\\x0b\\x0c]"),
            _ => {
                body.push('\\');
                body.push(next);
            }
        }
    }
    if casei {
        let positive = body
            .strip_prefix("[^")
            .map(|b| format!("[{b}"))
            .unwrap_or_else(|| body.clone());
        let re = regex::Regex::new(&format!("\\A{positive}\\z")).map_err(|e| e.to_string())?;
        let mut extra = String::new();
        for c in 'a'..='z' {
            if re.is_match(&c.to_string()) || re.is_match(&c.to_ascii_uppercase().to_string()) {
                extra.push(c);
                extra.push(c.to_ascii_uppercase());
            }
        }
        body.pop();
        body.push_str(&extra);
        body.push(']');
    }
    Ok(format!("(?-i:{body})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_accepts_the_entire_unicode_alphabet_without_aliasing() {
        let mut content = String::from("MARKéAéa");
        content.extend((128..=0x10ffff).filter_map(char::from_u32));
        for (pattern, expected) in [
            (r"(?i:MARK(?P<x>éa)(?P=x))", "éA"),
            (
                "([\u{10fffb}-\u{10fffd}]+)",
                "\u{10fffb}\u{10fffc}\u{10fffd}",
            ),
            (r"\b(MARK)\b", "MARK"),
            (r"(?a:MARK)(?u:(é))(?a:Aéa)", "é"),
        ] {
            let prepared = prepare(pattern, &content, true, false).unwrap();
            assert_eq!(prepared.text, content);
            let regex = fancy_regex::RegexBuilder::new(&prepared.pattern)
                .python_backreferences(true)
                .build()
                .unwrap();
            let captures = regex.captures(&prepared.text).unwrap().unwrap();
            assert_eq!(
                prepared.original(&content, captures.get(1).unwrap()),
                expected
            );
        }
        let prepared = prepare(r"(éa)\1", "éaÉa", true, true).unwrap();
        let regex = fancy_regex::RegexBuilder::new(&prepared.pattern)
            .case_insensitive(true)
            .python_backreferences(true)
            .build()
            .unwrap();
        assert!(!regex.is_match(&prepared.text).unwrap());
    }

    #[test]
    fn ascii_preserves_ranges_backreferences_and_capture_offsets() {
        for (pattern, content, expected) in [
            (r"([é-ê]+)", "😀èéêë", "éê"),
            (r"(éa)\1", "😀éaéa", "éa"),
            (
                r"([\uE000-\uE002]+)",
                "😀\u{e000}\u{e001}",
                "\u{e000}\u{e001}",
            ),
            (r"(\w+)", "éabcı", "abc"),
            (r"(?a:(\w+))", "éabcı", "abc"),
            (r"([\\u]+)", "é\\u", "\\u"),
        ] {
            let prepared = prepare(pattern, content, true, false).unwrap();
            let regex = fancy_regex::Regex::new(&prepared.pattern).unwrap();
            let captures = regex.captures(&prepared.text).unwrap().unwrap();
            assert_eq!(
                prepared.original(content, captures.get(1).unwrap()),
                expected,
                "{pattern}"
            );
        }
    }

    #[test]
    fn mixed_scopes_restore_classes_and_case_modes() {
        for (pattern, content, expected) in [
            (r"(?a:(\w+))(?u:\w+)", "abcé", "abcé"),
            (r"(?ai:[a-z]+)(?ui:[a-z]+)", "ABCİıſK", "ABCİıſK"),
            (r"(?i:(?a:abc)(?-i:XYZ))(?u:é)", "AbCXYZé", "AbCXYZé"),
            (r"(?a:[\W]+)(?u:a)", "éıa", "éıa"),
            (r"(?a:\babc\b)(?u:é)", "éabcé", "abcé"),
        ] {
            let prepared = prepare(pattern, content, false, false).unwrap();
            let re = fancy_regex::Regex::new(&prepared.pattern).unwrap();
            assert_eq!(
                re.find(&prepared.text).unwrap().unwrap().as_str(),
                expected,
                "{pattern}"
            );
        }
        for pattern in [r"(?ai:(a)\1)(?u:x)", r"(?ai:(?P<x>a)(?P=x))(?u:x)"] {
            let prepared = prepare(pattern, "aAx", false, false).unwrap();
            let re = fancy_regex::RegexBuilder::new(&prepared.pattern)
                .python_backreferences(true)
                .build()
                .unwrap();
            assert!(re.is_match(&prepared.text).unwrap());
        }
    }

    #[test]
    fn unicode_fold_and_explicit_scoped_mode_boundary() {
        for pattern in [r"([a-z]+)", r"(k+)", r"([\x61-\x7a]+)"] {
            let prepared = prepare(pattern, "İıſKABCkk", true, true).unwrap();
            let regex = fancy_regex::RegexBuilder::new(&prepared.pattern)
                .case_insensitive(true)
                .build()
                .unwrap();
            let found = regex.find(&prepared.text).unwrap().unwrap();
            assert!(found.as_str().is_ascii(), "{pattern}: {found:?}");
        }
        let prepared = prepare("([a-z]+)", "İıſKABC", false, true).unwrap();
        let regex = fancy_regex::RegexBuilder::new(&prepared.pattern)
            .case_insensitive(true)
            .build()
            .unwrap();
        assert_eq!(
            regex.find(&prepared.text).unwrap().unwrap().as_str(),
            "İıſKABC"
        );
        assert!(prepare(r"(?a:\w+)\w+", "x", false, false).is_ok());
        assert!(prepare("(?L)x", "x", false, false)
            .unwrap_err()
            .contains("LOCALE"));
    }
}
