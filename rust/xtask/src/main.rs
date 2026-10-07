//! Russet development tasks. Run them from the `rust` directory:
//!
//! ```text
//! cargo xtask package --target TARGET --bin-dir DIR [--output DIR]
//! cargo xtask promote --commit SHA --version VERSION --development-run ID --output DIR
//! cargo xtask licenses [--check]
//! ```

mod archive;
mod licenses;
mod package;
mod promote;

use std::{collections::BTreeMap, path::PathBuf, process::ExitCode};

const USAGE: &str = "usage:
  cargo xtask package --target TARGET --bin-dir DIR [--output DIR]
  cargo xtask promote --commit SHA --version VERSION --development-run ID --output DIR
  cargo xtask licenses [--check]

package  Archive native development binaries for one target. The archive is
         written to rust/dist unless --output is given; its path is printed.
promote  Check one successful four-target development run at COMMIT and turn
         its archives into release archives in --output. Writes local files
         only; GitHub metadata comes from the authenticated gh CLI.
licenses Collect the license files of every third-party crate the shipped
         binaries link into rust/licenses. With --check, fail if that folder
         is out of date instead of writing it.";

/// Parse `--name value` and `--name=value` options, allowing only `allowed`.
fn options(arguments: &[String], allowed: &[&str]) -> Result<BTreeMap<String, String>, String> {
    let mut parsed = BTreeMap::new();
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        let (name, value) = match argument.strip_prefix("--") {
            Some(option) => match option.split_once('=') {
                Some((name, value)) => (name.to_owned(), value.to_owned()),
                None => (
                    option.to_owned(),
                    iterator
                        .next()
                        .ok_or_else(|| format!("--{option} needs a value"))?
                        .clone(),
                ),
            },
            None => return Err(format!("unexpected argument: {argument}")),
        };
        if !allowed.contains(&name.as_str()) {
            return Err(format!("unknown option: --{name}"));
        }
        if parsed.insert(name.clone(), value).is_some() {
            return Err(format!("--{name} was given more than once"));
        }
    }
    Ok(parsed)
}

fn required(options: &mut BTreeMap<String, String>, name: &str) -> Result<String, String> {
    options
        .remove(name)
        .ok_or_else(|| format!("the --{name} option is required"))
}

fn package(arguments: &[String]) -> Result<ExitCode, String> {
    let mut options = options(arguments, &["target", "bin-dir", "output"])?;
    let target = required(&mut options, "target")?;
    if !archive::is_target(&target) {
        let names: Vec<_> = archive::TARGETS.iter().map(|(name, _, _)| *name).collect();
        return Err(format!("--target must be one of: {}", names.join(", ")));
    }
    let bin_dir = PathBuf::from(required(&mut options, "bin-dir")?);
    let root = package::repository_root();
    let output = options
        .remove("output")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("rust/dist"));
    match package::package(&root, &target, &bin_dir, &output) {
        Ok(path) => {
            println!("{}", path.display());
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("Packaging failed: {error}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn promote(arguments: &[String]) -> Result<ExitCode, String> {
    let mut options = options(
        arguments,
        &["commit", "version", "development-run", "output"],
    )?;
    let commit = required(&mut options, "commit")?;
    let version = required(&mut options, "version")?;
    let run = required(&mut options, "development-run")?;
    let run: u64 = run
        .parse()
        .map_err(|_| format!("--development-run must be a run ID, not {run}"))?;
    let output = PathBuf::from(required(&mut options, "output")?);
    match promote::promote(&promote::GitHub, &commit, &version, run, &output) {
        Ok(_) => {
            println!("{}", output.display());
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("Promotion refused: {error}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn licenses(arguments: &[String]) -> Result<ExitCode, String> {
    let check = match arguments {
        [] => false,
        [option] if option == "--check" => true,
        _ => return Err("licenses takes only --check".to_owned()),
    };
    match licenses::licenses(&package::repository_root(), check) {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(error) => {
            eprintln!("{error}");
            Ok(ExitCode::FAILURE)
        }
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.first().map(String::as_str) {
        Some("package") => package(&arguments[1..]),
        Some("promote") => promote(&arguments[1..]),
        Some("licenses") => licenses(&arguments[1..]),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err("expected a task: package, promote, or licenses".to_owned()),
    };
    result.unwrap_or_else(|error| {
        eprintln!("error: {error}\n\n{USAGE}");
        ExitCode::from(2)
    })
}
