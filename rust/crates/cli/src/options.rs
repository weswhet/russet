//! Native parsing of the frozen reference optparse contract.
use serde_json::Value;

pub enum Parsed {
    Arguments(Vec<String>),
    Exit(i32),
}
fn program() -> String {
    std::env::args()
        .next()
        .and_then(|p| {
            std::path::Path::new(&p)
                .file_name()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "russet".into())
}
fn render(text: &str) -> String {
    let program = program();
    text.replace("Usage: autopkg ", &format!("Usage: {program} "))
        .replace("Example: 'autopkg ", &format!("Example: '{program} "))
}
fn error(spec: &Value, message: &str) -> Parsed {
    autopkg_platform::text_eprintln!(
        "{}\n{}: error: {message}",
        render(spec["usage"].as_str().unwrap()),
        program()
    );
    Parsed::Exit(2)
}
/// Options Russet adds to a verb. They aren't in the captured parsers, so
/// `--help` stays the same as AutoPkg's. An `exact` option must be spelled
/// out, so abbreviations still match only AutoPkg's options.
fn extensions(verb: &str) -> Vec<Value> {
    match verb {
        "run" | "install" => vec![
            serde_json::json!({
                "flags": ["-j", "--jobs"],
                "takes_value": true,
            }),
            serde_json::json!({
                "flags": ["--plain"],
                "takes_value": false,
                "exact": true,
            }),
        ],
        _ => vec![],
    }
}
/// The long options `name` selects: the one spelled exactly, or every
/// non-exact option it abbreviates.
fn matching<'a>(options: &'a [Value], name: &str) -> Vec<(&'a str, &'a Value)> {
    let all: Vec<_> = options
        .iter()
        .flat_map(|o| {
            o["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(move |f| (f.as_str().unwrap(), o))
        })
        .collect();
    if let Some(exact) = all.iter().find(|(f, _)| *f == name) {
        vec![*exact]
    } else {
        all.into_iter()
            .filter(|(f, o)| f.starts_with(name) && o["exact"] != true)
            .collect()
    }
}
pub fn parse(verb: &str, args: &[String]) -> Parsed {
    let specs: Value = serde_json::from_str(include_str!(
        "../../../../compatibility/cli-parser-reference.json"
    ))
    .expect("frozen CLI parsers");
    let Some(spec) = specs["parsers"].get(verb) else {
        return Parsed::Arguments(args.to_vec());
    };
    let mut options = spec["options"].as_array().unwrap().clone();
    options.extend(extensions(verb));
    let options = &options;
    let mut output = vec![];
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        index += 1;
        if argument == "--" {
            output.extend(args[index - 1..].iter().cloned());
            break;
        }
        if !argument.starts_with('-') || argument == "-" {
            output.push(argument.clone());
            continue;
        }
        let mut pending = vec![];
        if argument.starts_with("--") {
            let (name, attached) = argument
                .split_once('=')
                .map(|(a, b)| (a, Some(b.to_owned())))
                .unwrap_or((argument.as_str(), None));
            let matching = matching(options, name);
            if matching.is_empty() {
                return error(spec, &format!("no such option: {name}"));
            }
            if matching.len() > 1 {
                let mut names: Vec<_> = matching.iter().map(|(f, _)| *f).collect();
                names.sort();
                return error(
                    spec,
                    &format!("ambiguous option: {name} ({}?)", names.join(", ")),
                );
            }
            pending.push((matching[0].0.to_owned(), matching[0].1, attached));
        } else {
            let chars = argument[1..].char_indices();
            for (offset, c) in chars {
                let name = format!("-{c}");
                let Some(option) = options.iter().find(|o| {
                    o["flags"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|f| f.as_str() == Some(&name))
                }) else {
                    return error(spec, &format!("no such option: {name}"));
                };
                let takes = option["takes_value"].as_bool().unwrap();
                let rest = &argument[1 + offset + c.len_utf8()..];
                pending.push((
                    name,
                    option,
                    if takes && !rest.is_empty() {
                        Some(rest.to_owned())
                    } else {
                        None
                    },
                ));
                if takes || c == 'h' {
                    break;
                }
            }
        }
        for (name, option, attached) in pending {
            let takes = option["takes_value"].as_bool().unwrap();
            if !takes && attached.is_some() {
                return error(spec, &format!("{name} option does not take a value"));
            }
            if name == "-h" || name == "--help" {
                autopkg_platform::text_print!("{}", render(spec["help"].as_str().unwrap()));
                return Parsed::Exit(0);
            }
            output.push(name.clone());
            if takes {
                let value = if let Some(v) = attached {
                    v
                } else if let Some(v) = args.get(index) {
                    index += 1;
                    v.clone()
                } else {
                    return error(spec, &format!("{name} option requires 1 argument"));
                };
                output.push(value);
            }
        }
    }
    Parsed::Arguments(output)
}
pub fn top_help(verb: Option<&str>) -> i32 {
    let contract = super::contract();
    let commands = contract["cli"]["subcommands"].as_object().unwrap();
    let longest = commands.keys().map(String::len).max().unwrap();
    let program = program();
    autopkg_platform::text_println!(
        "Usage: {program} <verb> <options>, where <verb> is one of the following:\n"
    );
    for (name, entry) in commands {
        autopkg_platform::text_println!(
            "    {name:longest$}  ({})",
            entry["help"].as_str().unwrap()
        );
    }
    autopkg_platform::text_println!();
    if let Some(verb) = verb.filter(|v| !commands.contains_key(*v)) {
        autopkg_platform::text_println!("Error: unknown verb: {verb}");
    } else {
        autopkg_platform::text_println!("{program} <verb> --help for more help for that verb");
    }
    1
}

pub fn usage_failure(verb: &str, message: Option<&str>, status: i32) -> Result<i32, String> {
    if let Some(message) = message {
        autopkg_platform::text_eprintln!("{message}");
    }
    let specs: Value = serde_json::from_str(include_str!(
        "../../../../compatibility/cli-parser-reference.json"
    ))
    .unwrap();
    autopkg_platform::text_eprintln!(
        "{}",
        render(specs["parsers"][verb]["usage"].as_str().unwrap())
    );
    Ok(status)
}
pub fn operands(verb: &str, args: &[String]) -> Vec<String> {
    let specs: Value = serde_json::from_str(include_str!(
        "../../../../compatibility/cli-parser-reference.json"
    ))
    .unwrap();
    let options = specs["parsers"][verb]["options"].as_array().unwrap();
    let mut result = vec![];
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            result.extend(args.cloned());
            break;
        }
        if let Some(option) = options.iter().find(|o| {
            o["flags"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some(arg))
        }) {
            if option["takes_value"] == true {
                args.next();
            }
        } else {
            result.push(arg.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(args: &[&str]) -> Option<Vec<String>> {
        let args: Vec<_> = args.iter().map(|&a| a.to_owned()).collect();
        match parse("run", &args) {
            Parsed::Arguments(output) => Some(output),
            Parsed::Exit(_) => None,
        }
    }

    #[test]
    fn plain_must_be_spelled_out() {
        assert_eq!(arguments(&["--plain", "A"]).unwrap(), ["--plain", "A"]);
        assert!(arguments(&["--plain=1"]).is_none());
        assert!(arguments(&["--pla"]).is_none());
        let mut options = extensions("run");
        options.push(serde_json::json!({"flags": ["--prefs"], "takes_value": true}));
        let names: Vec<_> = matching(&options, "--p").iter().map(|(f, _)| *f).collect();
        assert_eq!(names, ["--prefs"]);
    }
}
