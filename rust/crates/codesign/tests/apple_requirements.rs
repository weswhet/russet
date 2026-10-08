//! Compares the native verifier's requirement decisions with
//! `codesign --verify -R` on code that ships with macOS and on the Developer
//! ID fixture.
#![cfg(target_os = "macos")]

use russet_codesign::bundle::{verify, Options};
use russet_codesign::requirement::{Context, Requirement};
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

const REQUIREMENTS: [&str; 9] = [
    // SHA-1 of Apple Root CA, then of something else.
    "certificate root = H\"611e5b662c593a08ff58d14ae22452d198df6c60\"",
    "certificate root = H\"0000000000000000000000000000000000000000\"",
    "identifier com.apple.true and (anchor apple generic and certificate leaf[subject.OU] = X or certificate root = H\"611e5b662c593a08ff58d14ae22452d198df6c60\")",
    "anchor apple",
    "anchor apple generic",
    "anchor apple and identifier com.apple.true",
    "!anchor apple",
    "!(anchor apple generic)",
    "anchor apple generic and certificate leaf[subject.OU] = T4SK8ZXCXG",
];

fn apple(path: &Path, requirement: &str) -> bool {
    Command::new("/usr/bin/codesign")
        .args(["--verify", "-R"])
        .arg(format!("={requirement}"))
        .arg(path)
        .output()
        .unwrap()
        .status
        .success()
}

fn native(path: &Path, requirement: &str) -> bool {
    let signature = verify(
        path,
        Options {
            deep: false,
            strict: false,
        },
        SystemTime::now(),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Requirement::parse(requirement).unwrap().evaluate(&Context {
        identifier: &signature.identifier,
        chain: signature.chain.as_ref(),
        cdhashes: &signature.cdhashes,
    })
}

#[test]
fn requirement_decisions_match_codesign() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/MSCDockTilePlugin.docktileplugin");
    let paths = [
        Path::new("/usr/bin/true"),
        Path::new("/bin/ls"),
        Path::new("/usr/sbin/pkgutil"),
        fixture.as_path(),
    ];
    let mut differences = Vec::new();
    for path in paths {
        for requirement in REQUIREMENTS {
            let (expected, actual) = (apple(path, requirement), native(path, requirement));
            if expected != actual {
                differences.push(format!(
                    "{} {requirement:?}: codesign {expected}, native {actual}",
                    path.display()
                ));
            }
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    // The fixture is Developer ID signed, so it isn't Apple's own code.
    assert!(!native(&fixture, "anchor apple"));
    assert!(native(Path::new("/usr/bin/true"), "anchor apple"));
}
