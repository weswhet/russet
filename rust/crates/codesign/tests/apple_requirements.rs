//! Compares the native verifier's requirement decisions with
//! `codesign --verify -R` on code that ships with macOS and on the Developer
//! ID fixture.
#![cfg(target_os = "macos")]

use russet_codesign::bundle::{verify, Options};
use russet_codesign::requirement::Requirement;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

const REQUIREMENTS: [&str; 12] = [
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
    // EndNote X9's installer seals nested code with this.
    "anchor trusted",
    "!anchor trusted",
    "anchor trusted and certificate leaf[subject.OU] = T4SK8ZXCXG",
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
    signature.satisfies(&Requirement::parse(requirement).unwrap())
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

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A universal binary whose x86_64 slice keeps the developer's signature
/// and whose arm64 slice comes from an ad-hoc re-signing of the same bundle,
/// which seals the same resources: each architecture must satisfy the
/// requirement on its own, as `codesign` checks them.
#[test]
fn every_architecture_must_satisfy_the_requirement() {
    let temp = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/MSCDockTilePlugin.docktileplugin");
    let plugin = temp.path().join("Mixed.docktileplugin");
    let adhoc = temp.path().join("AdHoc.docktileplugin");
    for copy in [&plugin, &adhoc] {
        run(Command::new("/usr/bin/ditto").arg(&fixture).arg(copy));
    }
    run(Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&adhoc));
    let executable = "Contents/MacOS/MSCDockTilePlugin";
    let arm64 = temp.path().join("arm64");
    let x86_64 = temp.path().join("x86_64");
    for (source, arch, path) in [(&adhoc, "arm64", &arm64), (&plugin, "x86_64", &x86_64)] {
        run(Command::new("/usr/bin/lipo")
            .arg(source.join(executable))
            .args(["-thin", arch, "-output"])
            .arg(path));
    }
    run(Command::new("/usr/bin/lipo")
        .arg("-create")
        .arg(&x86_64)
        .arg(&arm64)
        .arg("-output")
        .arg(plugin.join(executable)));
    let requirement = REQUIREMENTS[8];
    assert!(native(&fixture, requirement));
    assert!(!apple(&plugin, requirement));
    // Each slice is validly signed, so the structure verifies; the
    // requirement mustn't.
    let signature = verify(
        &plugin,
        Options {
            deep: false,
            strict: false,
        },
        SystemTime::now(),
    )
    .unwrap();
    assert_eq!(signature.architectures.len(), 2);
    assert!(!signature.satisfies(&Requirement::parse(requirement).unwrap()));
}
