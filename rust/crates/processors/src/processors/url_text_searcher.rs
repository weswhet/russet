//! `URLTextSearcher`: download text with curl and search it with a Python
//! regular expression, saving the match and its named groups.
use super::url_getter::fetch;
use crate::{string, ExecutionFailure, Result};
use plist::{Dictionary, Value};
fn capture_close_order(pattern: &str) -> Result<Vec<usize>> {
    fn visit(expr: &fancy_regex::Expr, count: &mut usize, order: &mut Vec<usize>) {
        use fancy_regex::Expr;
        match expr {
            Expr::Group(child) => {
                *count += 1;
                let index = *count;
                visit(child, count, order);
                order.push(index);
            }
            Expr::Concat(children) | Expr::Alt(children) => {
                for child in children {
                    visit(child, count, order);
                }
            }
            Expr::LookAround(child, _) | Expr::AtomicGroup(child) | Expr::Repeat { child, .. } => {
                visit(child, count, order)
            }
            Expr::Conditional {
                condition,
                true_branch,
                false_branch,
            } => {
                visit(condition, count, order);
                visit(true_branch, count, order);
                visit(false_branch, count, order);
            }
            _ => (),
        }
    }
    let tree = fancy_regex::Expr::parse_tree_with_flags(
        pattern,
        fancy_regex::internal::FLAG_PYTHON_BACKREF,
    )
    .map_err(|e| e.to_string())?;
    let mut order = Vec::new();
    visit(&tree.expr, &mut 0, &mut order);
    Ok(order)
}
fn python_verbose(pattern: &str) -> String {
    let mut chars = pattern.chars().peekable();
    let mut output = String::new();
    let mut class = false;
    let mut class_first = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            output.push(c);
            if let Some(next) = chars.next() {
                output.push(next);
            }
            continue;
        }
        if class {
            output.push(c);
            if c == ']' && !class_first {
                class = false;
            }
            if !(class_first && c == '^') {
                class_first = false;
            }
            continue;
        }
        if c == '[' {
            class = true;
            class_first = true;
            output.push(c);
            continue;
        }
        if c == '#' {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
            continue;
        }
        if matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{b}' | '\u{c}') {
            continue;
        }
        output.push(c);
    }
    output
}
fn search(
    env: &mut Dictionary,
    content: &str,
) -> std::result::Result<Vec<String>, ExecutionFailure> {
    let original = string(env, "re_pattern")?;
    let verbose = env
        .get("re_flags")
        .and_then(Value::as_array)
        .is_some_and(|a| {
            a.iter()
                .any(|v| matches!(v.as_string(), Some("VERBOSE" | "X")))
        });
    let pattern = if verbose {
        python_verbose(original)
    } else {
        original.to_string()
    };
    let flags = env.get("re_flags").and_then(Value::as_array);
    let has = |names: &[&str]| {
        flags.is_some_and(|fs| {
            fs.iter()
                .any(|v| v.as_string().is_some_and(|v| names.contains(&v)))
        })
    };
    let ascii = has(&["ASCII", "A"]);
    let ignore_case = has(&["IGNORECASE", "I"]);
    if has(&["LOCALE", "L"]) {
        return Err(ExecutionFailure::unexpected(
            "cannot use LOCALE flag with a str pattern",
        ));
    }
    if ascii && has(&["UNICODE", "U"]) {
        return Err(ExecutionFailure::unexpected(
            "ASCII and UNICODE flags are incompatible",
        ));
    }
    let prepared = crate::python_regex::prepare(&pattern, content, ascii, ignore_case)
        .map_err(ExecutionFailure::unexpected)?;
    let mut builder = fancy_regex::RegexBuilder::new(&prepared.pattern);
    builder.python_backreferences(true);
    if let Some(flags) = env.get("re_flags") {
        for flag in flags.as_array().ok_or("re_flags must be an array")? {
            match flag.as_string().ok_or("re_flags entries must be strings")? {
                "IGNORECASE" | "I" => {
                    builder.case_insensitive(true);
                }
                "MULTILINE" | "M" => {
                    builder.multi_line(true);
                }
                "DOTALL" | "S" => {
                    builder.dot_matches_new_line(true);
                }
                "VERBOSE" | "X" => (),
                "UNICODE" | "U" | "NOFLAG" => (),
                "ASCII" | "A" => (),
                _ => (),
            }
        }
    }
    let regex = builder.build().map_err(|e| {
        ExecutionFailure::unexpected(format!(
            "Unsupported or invalid Python regular expression: {e}"
        ))
    })?;
    let captures = regex
        .captures(&prepared.text)
        .map_err(|e| ExecutionFailure::unexpected(e.to_string()))?
        .ok_or_else(|| {
            format!(
                "No match found on URL: {}",
                env.get("url").and_then(Value::as_string).unwrap_or("")
            )
        })?;
    let last = capture_close_order(&prepared.pattern)?
        .into_iter()
        .rev()
        .find_map(|i| captures.get(i))
        .or_else(|| captures.get(0))
        .unwrap();
    let output = env
        .get("result_output_var_name")
        .and_then(Value::as_string)
        .unwrap_or("match")
        .to_string();
    let mut results = Dictionary::new();
    for name in regex.capture_names().flatten() {
        let matched = captures
            .name(name)
            .map(|m| Value::String(prepared.original(content, m).into()))
            .unwrap_or(Value::Null);
        results.insert(name.to_string(), matched);
    }
    if !results.contains_key(&output) {
        results.insert(output, prepared.original(content, last).into());
    }
    for (key, value) in &results {
        autopkg_platform::processor_output(
            1,
            format!(
                "Found matching text ({key}): {}",
                value.as_string().unwrap_or("None")
            ),
        );
    }
    let output_names = results.keys().cloned().collect();
    for (key, value) in results {
        env.insert(key, value);
    }
    Ok(output_names)
}
pub(crate) fn execute(env: &mut Dictionary) -> Result<Vec<String>> {
    execute_typed(env).map_err(|e| e.message)
}
pub(crate) fn execute_typed(
    env: &mut Dictionary,
) -> std::result::Result<Vec<String>, ExecutionFailure> {
    let content = fetch(env)?;
    search(env, &content)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unmatched_named_group_preserves_python_none() {
        let mut env = Dictionary::from_iter([("re_pattern", "(?P<optional>a)?(?P<matched>b)")]);
        search(&mut env, "b").unwrap();
        assert!(env["optional"].is_null());
        assert_eq!(env["matched"].as_string(), Some("b"));
    }
    #[test]
    fn python_lastindex_nested_and_named() {
        let mut env = Dictionary::new();
        env.insert("re_pattern".into(), "((a)(b))".into());
        search(&mut env, "ab").unwrap();
        assert_eq!(env["match"].as_string(), Some("ab"));
        env.insert(
            "re_pattern".into(),
            "(?P<version>[0-9]+) (?P<match>[a-z]+)".into(),
        );
        search(&mut env, "42 file").unwrap();
        assert_eq!(env["version"].as_string(), Some("42"));
        assert_eq!(env["match"].as_string(), Some("file"));
    }
    #[test]
    fn curl_redirect_headers_and_text() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                // Accepted sockets inherit nonblocking mode on Windows.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_write_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 1024];
                loop {
                    let count = stream.read(&mut buf).unwrap();
                    request.extend_from_slice(&buf[..count]);
                    if count == 0 || request.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                assert!(String::from_utf8_lossy(&request).contains("X-Test: yes"));
                if index == 0 {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nversion=1.2.3").unwrap();
                }
            }
        });
        let mut env = Dictionary::new();
        env.insert("url".into(), format!("http://{address}/start").into());
        env.insert("re_pattern".into(), "version=(?P<version>[0-9.]+)".into());
        let mut headers = Dictionary::new();
        headers.insert("X-Test".into(), "yes".into());
        env.insert("request_headers".into(), headers.into());
        execute(&mut env).unwrap();
        assert_eq!(env["version"].as_string(), Some("1.2.3"));
        server.join().unwrap();
    }
    #[test]
    fn python_verbose_preserves_spaces_inside_character_classes() {
        let mut env = Dictionary::new();
        env.insert(
            "re_pattern".into(),
            "(?P<word>[ a-z]+) # comment with (ignored)\n (?P<tail>\\d+)".into(),
        );
        env.insert("re_flags".into(), Value::Array(vec!["VERBOSE".into()]));
        search(&mut env, "a b42").unwrap();
        assert_eq!(env["word"].as_string(), Some("a b"));
        assert_eq!(env["tail"].as_string(), Some("42"));
    }
    #[test]
    fn no_match_and_unsupported_regex_fail() {
        let mut env = Dictionary::new();
        env.insert("re_pattern".into(), "abc".into());
        assert_eq!(
            search(&mut env, "xyz").unwrap_err().kind,
            crate::FailureKind::Processor
        );
        env.insert("re_pattern".into(), "(?<=x)(y)".into());
        search(&mut env, "xy").unwrap();
        assert_eq!(env["match"].as_string(), Some("y"));
        env.insert("re_pattern".into(), r"(a+)\1".into());
        search(&mut env, "aaaa").unwrap();
        assert_eq!(env["match"].as_string(), Some("aa"));
        env.insert("re_pattern".into(), "(".into());
        assert_eq!(
            search(&mut env, "xy").unwrap_err().kind,
            crate::FailureKind::Unexpected
        );
    }
}
