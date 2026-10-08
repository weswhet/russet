//! Run every portable predicate fixture through `russet processor-run`
//! and compare the complete result with the frozen results that the pinned
//! AutoPkg reference produced with native NSPredicate on macOS.

use plist::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

const REFERENCE: &str = "c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c";

fn sorted(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            serde_json::Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&map[key])))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(sorted).collect())
        }
        other => other.clone(),
    }
}

/// The SHA-256 of the fixtures as compact JSON with sorted keys, which binds
/// the frozen results to the fixtures they were captured from.
fn fixture_digest(cases: &serde_json::Value) -> String {
    let text = serde_json::to_string(&sorted(cases)).unwrap();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn contains_null(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.iter().any(contains_null),
        Value::Dictionary(map) => map.values().any(contains_null),
        _ => false,
    }
}

/// Encode `value` as a binary plist that keeps null as the `0x00` object,
/// with dictionary keys sorted, as Python's `plistlib` writes it. The value
/// crate's writers store null as an empty string instead.
fn binary_plist(value: &Value) -> Vec<u8> {
    fn count(value: &Value) -> usize {
        1 + match value {
            Value::Array(items) => items.iter().map(count).sum(),
            Value::Dictionary(map) => map.len() + map.values().map(count).sum::<usize>(),
            _ => 0,
        }
    }
    fn marker(kind: u8, length: usize, out: &mut Vec<u8>) {
        if length < 15 {
            out.push(kind | length as u8);
        } else {
            out.push(kind | 0x0F);
            out.push(0x13);
            out.extend((length as u64).to_be_bytes());
        }
    }
    fn push(value: &Value, objects: &mut Vec<Vec<u8>>, reference: usize) -> usize {
        let index = objects.len();
        objects.push(Vec::new());
        let mut out = Vec::new();
        let refs = |items: Vec<usize>, out: &mut Vec<u8>| {
            for item in items {
                out.extend(&(item as u64).to_be_bytes()[8 - reference..]);
            }
        };
        match value {
            Value::Null => out.push(0x00),
            Value::Boolean(false) => out.push(0x08),
            Value::Boolean(true) => out.push(0x09),
            Value::Integer(integer) => {
                out.push(0x13);
                out.extend(integer.as_signed().unwrap().to_be_bytes());
            }
            Value::Real(real) => {
                out.push(0x23);
                out.extend(real.to_be_bytes());
            }
            Value::String(text) if text.is_ascii() => {
                marker(0x50, text.len(), &mut out);
                out.extend(text.as_bytes());
            }
            Value::String(text) => {
                let units: Vec<u16> = text.encode_utf16().collect();
                marker(0x60, units.len(), &mut out);
                for unit in units {
                    out.extend(unit.to_be_bytes());
                }
            }
            Value::Array(items) => {
                marker(0xA0, items.len(), &mut out);
                let children = items
                    .iter()
                    .map(|item| push(item, objects, reference))
                    .collect();
                refs(children, &mut out);
            }
            Value::Dictionary(map) => {
                marker(0xD0, map.len(), &mut out);
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                let names = keys
                    .iter()
                    .map(|key| push(&Value::String((*key).clone()), objects, reference))
                    .collect();
                let values = keys
                    .iter()
                    .map(|key| push(&map[key.as_str()], objects, reference))
                    .collect();
                refs(names, &mut out);
                refs(values, &mut out);
            }
            other => panic!("unsupported fixture value {other:?}"),
        }
        objects[index] = out;
        index
    }
    let total = count(value);
    let reference = if total < 256 { 1 } else { 2 };
    let mut objects = Vec::new();
    push(value, &mut objects, reference);
    let mut output = b"bplist00".to_vec();
    let mut offsets = Vec::new();
    for object in &objects {
        offsets.push(output.len() as u64);
        output.extend(object);
    }
    let table = output.len() as u64;
    for offset in &offsets {
        output.extend(&offset.to_be_bytes()[4..]);
    }
    output.extend([0, 0, 0, 0, 0, 0, 4, reference as u8]);
    output.extend((objects.len() as u64).to_be_bytes());
    output.extend(0u64.to_be_bytes());
    output.extend(table.to_be_bytes());
    output
}

#[test]
fn portable_predicates_match_frozen_native_results() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../compatibility/predicate-fixtures.json"
    ))
    .unwrap();
    let frozen: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../compatibility/predicate-reference-results.json"
    ))
    .unwrap();
    assert_eq!(frozen["reference_commit"], REFERENCE);
    assert_eq!(frozen["platform"], "darwin");
    assert_eq!(
        frozen["fixture_sha256"],
        fixture_digest(&cases).as_str(),
        "the frozen results don't match the current fixtures"
    );
    let temp = tempfile::tempdir().unwrap();
    let cases = cases.as_array().unwrap();
    assert!(!cases.is_empty());
    for (index, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        assert_eq!(case["processor"], "StopProcessingIf", "{name}");
        for key in case.as_object().unwrap().keys() {
            assert!(
                ["name", "processor", "environment", "files", "status"].contains(&key.as_str()),
                "{name}: this test doesn't support fixture key {key}"
            );
        }
        assert!(
            case["files"]
                .as_object()
                .is_none_or(|files| files.is_empty()),
            "{name}"
        );
        let expected = &frozen["cases"][name];
        let expected_status = case["status"].as_i64().unwrap_or(0);
        assert_eq!(expected["status"], expected_status, "{name}");
        let root = temp.path().join(format!("rust-{index}"));
        let home = temp.path().join(format!("rust-{index}-home"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(&home).unwrap();
        let environment: Value = serde_json::from_value(case["environment"].clone()).unwrap();
        let mut input = Vec::new();
        if contains_null(&environment) {
            input = binary_plist(&environment);
            assert_eq!(
                Value::from_reader(std::io::Cursor::new(&input)).unwrap(),
                environment
            );
        } else {
            environment.to_writer_xml(&mut input).unwrap();
        }
        let mut child = Command::new(env!("CARGO_BIN_EXE_russet"))
            .args(["processor-run", "StopProcessingIf"])
            .current_dir(&root)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("CFFIXED_USER_HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let output = child.wait_with_output().unwrap();
        let status = i64::from(output.status.code().unwrap());
        assert_eq!(
            status,
            expected_status,
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if status == 0 {
            let actual = Value::from_reader(std::io::Cursor::new(output.stdout)).unwrap();
            let expected: Value = serde_json::from_value(expected["environment"].clone()).unwrap();
            assert_eq!(actual, expected, "{name}");
        } else {
            assert!(expected["environment"].is_null(), "{name}");
        }
        assert!(
            expected["files"].as_object().unwrap().is_empty(),
            "{name}: frozen file results aren't supported"
        );
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            0,
            "{name}: the processor created files"
        );
    }
}
