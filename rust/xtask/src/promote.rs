//! `cargo xtask promote`: turn the archives from one successful four-target
//! development run into release archives.
//!
//! Promotion writes local files only. It never builds, installs, tags, or
//! publishes a release. GitHub metadata comes from the authenticated `gh` CLI.

use crate::archive::{
    is_apple, is_windows, read_tar_gz, read_zip, verify_binary, write_archive, Entries,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::Path,
    process::Command,
};

pub const REPOSITORY: &str = "weswhet/russet";
pub const REFERENCE: &str = "c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c";
/// Release targets and the GitHub-hosted runner image that builds each one.
pub const TARGETS: [(&str, &str); 4] = [
    ("aarch64-apple-darwin", "macos-15"),
    ("x86_64-apple-darwin", "macos-15-intel"),
    ("x86_64-unknown-linux-gnu", "ubuntu-24.04"),
    ("x86_64-pc-windows-msvc", "windows-2025"),
];
/// The workflow runs that must succeed at the release commit.
pub const WORKFLOWS: [(&str, &str); 1] = [("development", ".github/workflows/rust.yml")];

static NULL: Value = Value::Null;

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

fn require(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&map[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// Pretty JSON with sorted keys, two-space indentation, ASCII-only output,
/// and a final newline, the same bytes as Python's
/// `json.dumps(value, indent=2, sort_keys=True) + "\n"`.
pub fn json_bytes(value: &Value) -> Vec<u8> {
    let text = serde_json::to_string_pretty(&sorted(value)).expect("JSON values serialize");
    let mut output = String::with_capacity(text.len() + 1);
    for character in text.chars() {
        if character.is_ascii() && character != '\x7f' {
            output.push(character);
        } else {
            let mut units = [0u16; 2];
            for unit in character.encode_utf16(&mut units) {
                output.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    output.push('\n');
    output.into_bytes()
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && (parts[0] == "0" || !parts[0].starts_with('0'))
}

fn validate_job(job: &Value, required: &[String]) -> Result<(), String> {
    let name = text(field(job, "name"));
    require(
        field(job, "status") == "completed" && field(job, "conclusion") == "success",
        format!("Required job did not succeed: {name}"),
    )?;
    let steps: &[Value] = field(job, "steps").as_array().map_or(&[], Vec::as_slice);
    for step_name in required {
        let matches: Vec<_> = steps
            .iter()
            .filter(|step| field(step, "name") == step_name.as_str())
            .collect();
        require(
            matches.len() == 1 && field(matches[0], "conclusion") == "success",
            format!("Missing or unsuccessful step: {name}: {step_name}"),
        )?;
    }
    require(
        steps.iter().all(|step| {
            let conclusion = field(step, "conclusion");
            conclusion == "success" || conclusion == "skipped"
        }),
        format!("Unsuccessful step in job: {name}"),
    )
}

/// The step names that every development job must complete successfully.
/// They follow `.github/workflows/rust.yml`.
pub fn development_steps(target: &str) -> Vec<String> {
    let mut required = vec![
        "Run cargo fmt --all -- --check".to_owned(),
        "Run cargo clippy --workspace --all-targets --locked -- -D warnings".to_owned(),
        format!(
            "Run cargo test --workspace --exclude xtask --locked --no-fail-fast --target {target}"
        ),
        "Verify archive install, upgrade, rollback, and failure recovery".to_owned(),
        format!("Run cargo build --release --locked --target {target} -p russet"),
        format!("Run cargo xtask package --target {target} --bin-dir target/{target}/release"),
        "Run actions/upload-artifact@v4".to_owned(),
    ];
    required.push(
        if is_apple(target) {
            "Verify installed macOS helpers, upgrade, and rollback"
        } else if is_windows(target) {
            "Verify native Windows workflows"
        } else {
            "Verify Linux installation without Python"
        }
        .to_owned(),
    );
    required
}

/// Every job in `.github/workflows/rust.yml`: one test job per target, and
/// the fuzz job.
fn expected_jobs() -> BTreeMap<String, Vec<String>> {
    let mut jobs: BTreeMap<_, _> = TARGETS
        .iter()
        .map(|(target, runner)| {
            (
                format!("test ({runner}, {target})"),
                development_steps(target),
            )
        })
        .collect();
    jobs.insert(
        "fuzz".to_owned(),
        vec!["Run each target for 60 seconds".to_owned()],
    );
    jobs
}

pub fn validate_gates(sha: &str, gates: &Map<String, Value>) -> Result<(), String> {
    require(
        is_lower_hex(sha, 40),
        "A full lowercase source commit SHA is required",
    )?;
    let keys: BTreeSet<_> = gates.keys().map(String::as_str).collect();
    let required: BTreeSet<_> = WORKFLOWS.iter().map(|(key, _)| *key).collect();
    require(
        keys == required,
        "The development workflow gate is required",
    )?;
    for (key, path) in WORKFLOWS {
        let run = field(&gates[key], "run");
        let jobs: &[Value] = field(&gates[key], "jobs")
            .as_array()
            .map_or(&[], Vec::as_slice);
        require(
            field(field(run, "repository"), "full_name") == REPOSITORY
                && field(field(run, "head_repository"), "full_name") == REPOSITORY,
            "Gate repository does not match the release repository",
        )?;
        require(
            field(run, "path") == path && field(run, "head_sha") == sha,
            format!("Gate workflow or commit mismatch: {key}"),
        )?;
        let event = field(run, "event");
        require(
            event == "push" || event == "workflow_dispatch",
            "Pull-request gates cannot promote releases",
        )?;
        require(
            field(run, "status") == "completed" && field(run, "conclusion") == "success",
            format!("Gate did not succeed: {key}"),
        )?;
        let attempt = field(run, "run_attempt");
        require(
            attempt.as_u64().is_some_and(|attempt| attempt > 0),
            "Missing gate attempt",
        )?;
        require(
            jobs.iter().all(|job| {
                field(job, "head_sha") == sha
                    && field(job, "run_id") == field(run, "id")
                    && field(job, "run_attempt") == attempt
            }),
            "Job provenance mismatch",
        )?;
        let expected = expected_jobs();
        let names: BTreeSet<String> = jobs.iter().map(|job| text(field(job, "name"))).collect();
        require(
            jobs.len() == expected.len()
                && names == expected.keys().cloned().collect::<BTreeSet<_>>(),
            format!("Required job set mismatch: {key}"),
        )?;
        for job in jobs {
            validate_job(job, &expected[&text(field(job, "name"))])?;
        }
    }
    Ok(())
}

fn timestamp(value: &Value) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    let text = value.as_str().ok_or("Artifact/job timestamp is missing")?;
    chrono::DateTime::parse_from_rfc3339(text).map_err(|_| {
        if chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").is_ok() {
            "Artifact/job timestamp must include timezone".to_owned()
        } else {
            format!("Artifact/job timestamp is invalid: {text}")
        }
    })
}

/// Reject an artifact retained from an earlier attempt of a rerun job.
pub fn validate_artifact_attempt(artifact: &Value, job: &Value) -> Result<(), String> {
    let created = timestamp(field(artifact, "created_at"))?;
    require(
        timestamp(field(job, "started_at"))? <= created
            && created <= timestamp(field(job, "completed_at"))?,
        "Artifact was not created during the successful job attempt",
    )
}

pub fn validate_artifacts(sha: &str, run_id: u64, artifacts: &[Value]) -> Result<(), String> {
    let expected: BTreeSet<String> = TARGETS
        .iter()
        .map(|(target, _)| format!("russet-development-{target}"))
        .collect();
    let names: BTreeSet<String> = artifacts
        .iter()
        .map(|artifact| text(field(artifact, "name")))
        .collect();
    require(
        artifacts.len() == 4 && names == expected,
        "Exactly four unique target artifacts are required",
    )?;
    for artifact in artifacts {
        require(field(artifact, "expired") == false, "Artifact has expired")?;
        let run = field(artifact, "workflow_run");
        require(
            field(run, "id").as_u64() == Some(run_id) && field(run, "head_sha") == sha,
            "Artifact provenance mismatch",
        )?;
        let digest = field(artifact, "digest").as_str().unwrap_or("");
        require(
            digest
                .strip_prefix("sha256:")
                .is_some_and(|hex| is_lower_hex(hex, 64)),
            "Artifact SHA-256 digest is required",
        )?;
    }
    Ok(())
}

/// Return the normalized path components of a relative archive member name,
/// rejecting anything that could escape the extraction directory.
pub fn safe_name(name: &str) -> Result<Vec<&str>, String> {
    let parts: Vec<&str> = name
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let normalized = if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    };
    require(
        !name.is_empty()
            && !name.starts_with('/')
            && !parts.contains(&"..")
            && !name.contains('\\')
            && normalized == name
            && !name.contains(':'),
        format!("Unsafe archive path: {name}"),
    )?;
    Ok(parts)
}

/// Read and validate one development archive's members, keyed by their path
/// below the archive root.
pub fn read_payload(data: &[u8], target: &str) -> Result<Entries, String> {
    let root = format!("russet-development-{target}");
    let members = if is_windows(target) {
        read_zip(data)?
    } else {
        read_tar_gz(data)?
    };
    let mut entries = Entries::new();
    for member in members {
        require(member.regular, "Only regular archive files are permitted")?;
        let parts = safe_name(&member.name)?;
        require(
            parts.len() > 1 && parts[0] == root,
            "Unexpected archive root",
        )?;
        let relative = parts[1..].join("/");
        require(!entries.contains_key(&relative), "Duplicate archive member")?;
        require(
            member.mode & !0o777 == 0,
            "Special permission bits are not permitted",
        )?;
        entries.insert(relative, (member.data, member.mode));
    }
    let suffix = if is_windows(target) { ".exe" } else { "" };
    let binary = format!("bin/russet{suffix}");
    let mut expected = BTreeSet::from([binary.clone()]);
    expected.extend(["README.md", "INSTALL.md", "LICENSE.txt"].map(String::from));
    expected.insert(
        if is_windows(target) {
            "install.ps1"
        } else {
            "install.sh"
        }
        .into(),
    );
    if is_apple(target) {
        expected.extend(
            [
                "launchd/russet-server.plist",
                "launchd/russet-installd.plist",
            ]
            .map(String::from),
        );
    }
    require(
        expected.iter().all(|name| entries.contains_key(name)),
        "Archive is missing a required payload",
    )?;
    require(
        entries
            .keys()
            .all(|name| expected.contains(name) || name.starts_with("licenses/")),
        "Unexpected archive payload",
    )?;
    require(
        entries.keys().any(|name| name.starts_with("licenses/")),
        "Third-party licenses are missing",
    )?;
    verify_binary(&entries[&binary].0, target)?;
    require(
        entries[&binary].1 == 0o755,
        "Executable permissions changed",
    )?;
    Ok(entries)
}

/// The README.md and INSTALL.md that replace the development documents in a
/// release archive.
pub fn release_documents(version: &str, sha: &str) -> Entries {
    let readme = format!(
        "# Russet {version}

This verified native distribution implements the AutoPkg 3.0.0 compatibility
interface. Its distribution version is {version}; `russet version` remains
3.0.0 for recipe compatibility. Source commit: `{sha}`.

The binaries and installers are unchanged from the four-platform development
run recorded in RELEASE.json. That run built these archives and checked them
through native installation, workflow, upgrade, and rollback tests on each
target.

See INSTALL.md for installation and rollback. RELEASE.json records the gate
run and original payload hashes. Verify the downloaded archive against the
release SHA256SUMS before installation. No Python runtime is bundled or needed.
DEVELOPMENT-README.md and DEVELOPMENT-INSTALL.md preserve the original
candidate documentation as historical context, not this release's status.
"
    );
    let install = r"# Installation and rollback

Extract the archive for your platform. Keep the extracted directory available
for rollback. Installers preserve the previous installation in a sibling
rollback directory and install the native command as `russet`. On macOS and
Linux, Russet installs into `/opt/russet` and links `/usr/local/bin/russet`.
It doesn't change an existing Python AutoPkg installation.

macOS and Linux (from the extracted directory):

```sh
sudo /bin/sh ./install.sh install
sudo /bin/sh ./install.sh rollback
```

Windows PowerShell (choose an installation directory whose parent exists):

```powershell
./install.ps1 install -Destination 'C:\Tools\Russet'
./install.ps1 rollback -Destination 'C:\Tools\Russet'
```

Windows does not update PATH or Chocolatey shims. Invoke the installed
`russet.exe` directly or configure PATH yourself. After an interrupted Windows
transaction, run `./install.ps1 recover -Destination 'C:\Tools\Russet'`
from this extracted archive before retrying.

On macOS, launchd starts the helper services as `russet --server` and
`russet --installd`, with their own job names and sockets, so they don't
conflict with Python AutoPkg's services. Preferences and recipes retain their
existing formats. Retain rollback generations until you have verified your own recipes. Detailed staging
options and transaction behavior are preserved in DEVELOPMENT-INSTALL.md.
";
    Entries::from([
        ("README.md".to_owned(), (readme.into_bytes(), 0o644)),
        (
            "INSTALL.md".to_owned(),
            (install.as_bytes().to_vec(), 0o644),
        ),
    ])
}

/// The GitHub REST API calls that promotion needs.
pub trait Api {
    fn get(&self, endpoint: &str) -> Result<Value, String>;
    fn raw(&self, endpoint: &str) -> Result<Vec<u8>, String>;
    /// Read every page of a list endpoint that wraps its items in `key`.
    fn pages(&self, endpoint: &str, key: &str) -> Result<Vec<Value>, String> {
        let mut result = Vec::new();
        for page in 1.. {
            let data = self.get(&format!("{endpoint}?per_page=100&page={page}"))?;
            let items = data
                .get(key)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("GitHub response is missing {key}"))?;
            result.extend(items.iter().cloned());
            if items.len() < 100 {
                break;
            }
        }
        Ok(result)
    }
}

/// The authenticated `gh` CLI.
pub struct GitHub;

impl Api for GitHub {
    fn get(&self, endpoint: &str) -> Result<Value, String> {
        serde_json::from_slice(&self.raw(endpoint)?)
            .map_err(|error| format!("GitHub returned invalid JSON for {endpoint}: {error}"))
    }

    fn raw(&self, endpoint: &str) -> Result<Vec<u8>, String> {
        let output = Command::new("gh")
            .args(["api", "--method", "GET", endpoint])
            .output()
            .map_err(|error| format!("Could not run gh: {error}"))?;
        require(
            output.status.success(),
            format!(
                "gh api {endpoint} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )?;
        Ok(output.stdout)
    }
}

fn unwrap_single(wrapper: &[u8], expected: &str, message: &str) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(wrapper))
        .map_err(|error| format!("Could not read ZIP archive: {error}"))?;
    require(archive.len() == 1, message)?;
    let mut file = archive
        .by_index(0)
        .map_err(|error| format!("Could not read ZIP archive: {error}"))?;
    require(file.name() == expected, message)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)
        .map_err(|error| format!("Could not read ZIP archive: {error}"))?;
    Ok(data)
}

/// Validate the development run for `sha`, then write the release archives,
/// `gate-evidence.json`, and `SHA256SUMS` to `output`. Nothing appears at
/// `output` unless every check passes.
pub fn promote(
    api: &dyn Api,
    sha: &str,
    version: &str,
    development_run: u64,
    output: &Path,
) -> Result<Value, String> {
    require(
        is_version(version),
        "Distribution version must be MAJOR.MINOR.PATCH",
    )?;
    require(
        fs::symlink_metadata(output).is_err(),
        "Output directory must not already exist",
    )?;
    let prefix = format!("repos/{REPOSITORY}/actions");
    let run = api.get(&format!("{prefix}/runs/{development_run}"))?;
    require(
        field(&run, "id").as_u64() == Some(development_run),
        "Run ID mismatch",
    )?;
    let attempt = text(field(&run, "run_attempt"));
    let jobs = api.pages(
        &format!("{prefix}/runs/{development_run}/attempts/{attempt}/jobs"),
        "jobs",
    )?;
    let mut gates = Map::new();
    gates.insert(WORKFLOWS[0].0.into(), json!({"run": run, "jobs": jobs}));
    validate_gates(sha, &gates)?;
    let artifacts = api.pages(
        &format!("{prefix}/runs/{development_run}/artifacts"),
        "artifacts",
    )?;
    validate_artifacts(sha, development_run, &artifacts)?;
    let jobs = gates["development"]["jobs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let target_of = |artifact: &Value| {
        text(field(artifact, "name"))
            .trim_start_matches("russet-development-")
            .to_owned()
    };
    for artifact in &artifacts {
        let target = target_of(artifact);
        let runner = TARGETS
            .iter()
            .find(|(name, _)| *name == target)
            .map(|(_, runner)| *runner)
            .ok_or("Unexpected artifact target")?;
        let name = format!("test ({runner}, {target})");
        let job = jobs
            .iter()
            .find(|job| field(job, "name") == name.as_str())
            .ok_or("Artifact job is missing")?;
        validate_artifact_attempt(artifact, job)?;
    }
    let evidence = json!({
        "schema_version": 1,
        "source_commit": sha,
        "repository": REPOSITORY,
        "distribution_version": version,
        "compatibility_version": "3.0.0",
        "reference_commit": REFERENCE,
        "gates": gates,
        "artifacts": artifacts,
        "validation": {
            "installation": "exact development release archives on native target runners",
        },
    });
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    // Publish the directory only after every artifact and gate has validated.
    let temp = tempfile::Builder::new()
        .prefix(".russet-promotion-")
        .tempdir_in(parent)
        .map_err(|error| format!("{}: {error}", parent.display()))?;
    let staging = temp.path().join("release");
    let io = |error: std::io::Error| format!("{}: {error}", staging.display());
    fs::create_dir(&staging).map_err(io)?;
    let mut ordered = artifacts.clone();
    ordered.sort_by_key(|artifact| text(field(artifact, "name")));
    for artifact in &ordered {
        let target = target_of(artifact);
        let wrapper = api.raw(&format!("{prefix}/artifacts/{}/zip", field(artifact, "id")))?;
        require(
            format!("sha256:{}", digest(&wrapper)) == text(field(artifact, "digest")),
            "Downloaded artifact digest mismatch",
        )?;
        let extension = if is_windows(&target) {
            ".zip"
        } else {
            ".tar.gz"
        };
        let original_name = format!("{}{extension}", text(field(artifact, "name")));
        let data = unwrap_single(&wrapper, &original_name, "Unexpected CI artifact contents")?;
        let mut entries = read_payload(&data, &target)?;
        let payload_hashes: Map<String, Value> = entries
            .iter()
            .map(|(name, (content, mode))| {
                (
                    name.clone(),
                    json!({"sha256": digest(content), "mode": mode}),
                )
            })
            .collect();
        for name in ["README.md", "INSTALL.md"] {
            let original = entries.remove(name).expect("validated payload");
            entries.insert(format!("DEVELOPMENT-{name}"), original);
        }
        entries.extend(release_documents(version, sha));
        let mut metadata: Map<String, Value> = evidence
            .as_object()
            .expect("evidence is an object")
            .iter()
            .filter(|(key, _)| *key != "gates" && *key != "artifacts")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        metadata.insert("target".into(), json!(target));
        metadata.insert("original_archive_sha256".into(), json!(digest(&data)));
        metadata.insert("original_artifact".into(), artifact.clone());
        metadata.insert("original_payloads".into(), Value::Object(payload_hashes));
        let summary: Map<String, Value> = gates
            .iter()
            .map(|(key, gate)| {
                let run = &gate["run"];
                let fields: Map<String, Value> =
                    ["id", "run_attempt", "head_sha", "path", "html_url"]
                        .iter()
                        .map(|name| ((*name).to_owned(), field(run, name).clone()))
                        .collect();
                (key.clone(), Value::Object(fields))
            })
            .collect();
        metadata.insert("gates".into(), Value::Object(summary));
        entries.insert(
            "RELEASE.json".into(),
            (json_bytes(&Value::Object(metadata)), 0o644),
        );
        let root = format!("russet-{version}-{target}");
        write_archive(&staging.join(format!("{root}{extension}")), &root, &entries)?;
    }
    fs::write(staging.join("gate-evidence.json"), json_bytes(&evidence)).map_err(io)?;
    let mut names: Vec<_> = fs::read_dir(&staging)
        .map_err(io)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<_, _>>()
        .map_err(io)?;
    names.sort();
    let mut checksums = String::new();
    for name in names {
        let data = fs::read(staging.join(&name)).map_err(io)?;
        checksums.push_str(&format!("{}  {}\n", digest(&data), name.to_string_lossy()));
    }
    fs::write(staging.join("SHA256SUMS"), checksums).map_err(io)?;
    fs::rename(&staging, output).map_err(|error| format!("{}: {error}", output.display()))?;
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::tests::executable;
    use std::{cell::RefCell, collections::HashMap, io::Write};

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn fixture() -> Map<String, Value> {
        let run = json!({
            "id": 1, "path": WORKFLOWS[0].1, "head_sha": SHA, "run_attempt": 1,
            "event": "workflow_dispatch", "status": "completed", "conclusion": "success",
            "repository": {"full_name": REPOSITORY},
            "head_repository": {"full_name": REPOSITORY},
            "html_url": format!("https://github.com/{REPOSITORY}/actions/runs/1"),
        });
        let jobs: Vec<Value> = expected_jobs()
            .into_iter()
            .enumerate()
            .map(|(index, (name, steps))| {
                json!({
                    "id": 100 + index, "name": name, "status": "completed",
                    "conclusion": "success", "head_sha": SHA, "run_id": 1, "run_attempt": 1,
                    "started_at": "2026-01-01T00:00:00Z", "completed_at": "2026-01-01T00:30:00Z",
                    "steps": steps.iter().map(|step| json!({"name": step, "conclusion": "success"})).collect::<Vec<_>>(),
                })
            })
            .collect();
        Map::from_iter([("development".to_owned(), json!({"run": run, "jobs": jobs}))])
    }

    fn payload(target: &str) -> Entries {
        let suffix = if is_windows(target) { ".exe" } else { "" };
        let mut entries = Entries::from([
            (format!("bin/russet{suffix}"), (executable(target), 0o755)),
            (
                "README.md".into(),
                (b"original development README".to_vec(), 0o644),
            ),
            (
                "INSTALL.md".into(),
                (b"original candidate instructions".to_vec(), 0o644),
            ),
            ("LICENSE.txt".into(), (b"license".to_vec(), 0o644)),
            (
                "licenses/dependency/LICENSE".into(),
                (b"dependency license".to_vec(), 0o644),
            ),
        ]);
        if is_windows(target) {
            entries.insert(
                "install.ps1".into(),
                (b"installer bytes\x00\xff".to_vec(), 0o644),
            );
        } else {
            entries.insert(
                "install.sh".into(),
                (b"installer bytes\x00\xff".to_vec(), 0o755),
            );
        }
        if is_apple(target) {
            for name in ["russet-server", "russet-installd"] {
                entries.insert(
                    format!("launchd/{name}.plist"),
                    (b"launchd bytes".to_vec(), 0o644),
                );
            }
        }
        entries
    }

    fn zip_wrapper(name: &str, data: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
        writer.finish().unwrap().into_inner()
    }

    struct FakeApi {
        gates: Map<String, Value>,
        artifacts: Vec<Value>,
        raws: RefCell<HashMap<u64, Vec<u8>>>,
    }

    impl FakeApi {
        fn new(directory: &Path) -> Self {
            let mut artifacts = Vec::new();
            let mut raws = HashMap::new();
            for (index, (target, _)) in TARGETS.iter().enumerate() {
                let id = 10 + index as u64;
                let name = format!("russet-development-{target}");
                let extension = if is_windows(target) {
                    ".zip"
                } else {
                    ".tar.gz"
                };
                let path = directory.join(format!("{name}{extension}"));
                write_archive(&path, &name, &payload(target)).unwrap();
                let data = zip_wrapper(&format!("{name}{extension}"), &fs::read(&path).unwrap());
                artifacts.push(json!({
                    "id": id, "name": name, "expired": false,
                    "digest": format!("sha256:{}", digest(&data)),
                    "workflow_run": {"id": 1, "head_sha": SHA},
                    "created_at": "2026-01-01T00:20:00Z",
                }));
                raws.insert(id, data);
            }
            Self {
                gates: fixture(),
                artifacts,
                raws: RefCell::new(raws),
            }
        }
    }

    impl Api for FakeApi {
        fn get(&self, endpoint: &str) -> Result<Value, String> {
            let number: u64 = endpoint.rsplit('/').next().unwrap().parse().unwrap();
            Ok(self
                .gates
                .values()
                .find(|gate| gate["run"]["id"] == number)
                .map(|gate| gate["run"].clone())
                .unwrap())
        }
        fn pages(&self, _endpoint: &str, key: &str) -> Result<Vec<Value>, String> {
            Ok(if key == "artifacts" {
                self.artifacts.clone()
            } else {
                self.gates["development"]["jobs"]
                    .as_array()
                    .unwrap()
                    .clone()
            })
        }
        fn raw(&self, endpoint: &str) -> Result<Vec<u8>, String> {
            let id: u64 = endpoint
                .split("/artifacts/")
                .nth(1)
                .unwrap()
                .split('/')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            Ok(self.raws.borrow()[&id].clone())
        }
    }

    fn entries_of(path: &Path) -> Entries {
        let data = fs::read(path).unwrap();
        let members = if path.extension().unwrap() == "zip" {
            read_zip(&data).unwrap()
        } else {
            read_tar_gz(&data).unwrap()
        };
        members
            .into_iter()
            .map(|m| {
                (
                    m.name.split_once('/').unwrap().1.to_owned(),
                    (m.data, m.mode),
                )
            })
            .collect()
    }

    #[test]
    fn required_gate_accepts_only_exact_success() {
        validate_gates(SHA, &fixture()).unwrap();
        type Change = fn(&mut Map<String, Value>);
        let changes: [Change; 11] = [
            |g| g["development"]["run"]["head_sha"] = json!("b".repeat(40)),
            |g| g["development"]["run"]["path"] = json!(".github/workflows/rust-release.yml"),
            |g| g["development"]["run"]["conclusion"] = json!("failure"),
            |g| g["development"]["run"]["status"] = json!("in_progress"),
            |g| g["development"]["run"]["event"] = json!("pull_request"),
            |g| g["development"]["run"]["head_repository"]["full_name"] = json!("other/repo"),
            |g| g["development"]["jobs"][0]["run_attempt"] = json!(2),
            |g| g["development"]["jobs"][0]["conclusion"] = json!("skipped"),
            |g| {
                g["development"]["jobs"].as_array_mut().unwrap().pop();
            },
            |g| {
                g["development"]["jobs"][0]["steps"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            },
            |g| {
                g.remove("development");
            },
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut gates = fixture();
            change(&mut gates);
            assert!(validate_gates(SHA, &gates).is_err(), "change {index}");
        }
        let mut gates = fixture();
        gates.insert("differential".into(), gates["development"].clone());
        assert!(
            validate_gates(SHA, &gates).is_err(),
            "unexpected extra gate"
        );
        assert!(validate_gates(&SHA.to_uppercase(), &fixture()).is_err());
    }

    #[test]
    fn platform_verification_steps_cannot_be_skipped() {
        for (target, runner) in TARGETS {
            let name = development_steps(target).pop().unwrap();
            let mut gates = fixture();
            let job = gates["development"]["jobs"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|job| job["name"] == format!("test ({runner}, {target})"))
                .unwrap();
            let step = job["steps"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|step| step["name"] == name.as_str())
                .unwrap();
            step["conclusion"] = json!("skipped");
            let error = validate_gates(SHA, &gates).unwrap_err();
            assert!(error.contains(&name), "{target}: {error}");
        }
    }

    #[test]
    fn artifact_identity_digest_expiry_and_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let api = FakeApi::new(directory.path());
        validate_artifacts(SHA, 1, &api.artifacts).unwrap();
        for (key, value) in [
            ("expired", json!(true)),
            ("digest", json!("")),
            ("name", json!("wrong")),
        ] {
            let mut artifacts = api.artifacts.clone();
            artifacts[0][key] = value;
            assert!(validate_artifacts(SHA, 1, &artifacts).is_err(), "{key}");
        }
        let mut artifacts = api.artifacts.clone();
        artifacts[0].as_object_mut().unwrap().remove("expired");
        assert!(
            validate_artifacts(SHA, 1, &artifacts).is_err(),
            "missing expiry"
        );
        assert!(validate_artifacts(SHA, 2, &api.artifacts).is_err());
        let mut duplicated = api.artifacts[..3].to_vec();
        duplicated.push(api.artifacts[0].clone());
        assert!(validate_artifacts(SHA, 1, &duplicated).is_err());
    }

    #[test]
    fn artifact_must_come_from_the_successful_attempt() {
        let job =
            json!({"started_at": "2026-01-01T00:00:00Z", "completed_at": "2026-01-01T00:30:00Z"});
        validate_artifact_attempt(&json!({"created_at": "2026-01-01T01:20:00+01:00"}), &job)
            .unwrap();
        for created in ["2025-12-31T23:59:00Z", "2026-01-01T00:31:00Z"] {
            let error =
                validate_artifact_attempt(&json!({"created_at": created}), &job).unwrap_err();
            assert_eq!(
                error,
                "Artifact was not created during the successful job attempt"
            );
        }
        let error = validate_artifact_attempt(&json!({"created_at": "2026-01-01T00:20:00"}), &job)
            .unwrap_err();
        assert_eq!(error, "Artifact/job timestamp must include timezone");
        assert!(validate_artifact_attempt(&json!({}), &job).is_err());
    }

    #[test]
    fn archive_rejects_unsafe_members_and_wrong_architecture() {
        for name in [
            "../outside",
            "/absolute",
            "a/../../b",
            "a\\b",
            "a//b",
            "a/./b",
            "c:d",
            "",
        ] {
            assert!(safe_name(name).is_err(), "{name}");
        }
        let target = "x86_64-unknown-linux-gnu";
        let root = format!("russet-development-{target}");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.tar.gz");
        for mutation in [
            "binary",
            "duplicate",
            "symlink",
            "root",
            "extra",
            "permission",
            "missing",
        ] {
            let mut entries = payload(target);
            match mutation {
                "binary" => {
                    entries.insert("bin/russet".into(), (b"not ELF".to_vec(), 0o755));
                }
                "extra" => {
                    entries.insert("unknown".into(), (b"extra".to_vec(), 0o644));
                }
                "permission" => {
                    let data = entries["bin/russet"].0.clone();
                    entries.insert("bin/russet".into(), (data, 0o4755));
                }
                "missing" => {
                    entries.remove("LICENSE.txt");
                }
                _ => {}
            }
            write_archive(
                &path,
                if mutation == "root" { "wrong" } else { &root },
                &entries,
            )
            .unwrap();
            if mutation == "duplicate" || mutation == "symlink" {
                // Append a second README.md member, or a symbolic link, with
                // the tar crate so it bypasses the deterministic writer.
                let file = fs::File::create(&path).unwrap();
                let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
                let mut builder = tar::Builder::new(encoder);
                for (name, (data, mode)) in &entries {
                    let mut header = tar::Header::new_ustar();
                    header.set_size(data.len() as u64);
                    header.set_mode(*mode);
                    builder
                        .append_data(&mut header, format!("{root}/{name}"), data.as_slice())
                        .unwrap();
                }
                let mut header = tar::Header::new_ustar();
                header.set_size(0);
                if mutation == "symlink" {
                    header.set_entry_type(tar::EntryType::Symlink);
                    header.set_link_name("/outside").unwrap();
                }
                builder
                    .append_data(&mut header, format!("{root}/README.md"), [].as_slice())
                    .unwrap();
                builder.into_inner().unwrap().finish().unwrap();
            }
            assert!(
                read_payload(&fs::read(&path).unwrap(), target).is_err(),
                "{mutation}"
            );
        }
        write_archive(&path, &root, &payload(target)).unwrap();
        read_payload(&fs::read(&path).unwrap(), target).unwrap();
    }

    #[test]
    fn four_archives_preserve_payload_and_are_reproducible() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        let api = FakeApi::new(base);
        let outputs = [base.join("first"), base.join("second")];
        for output in &outputs {
            promote(&api, SHA, "0.1.0", 1, output).unwrap();
        }
        let files = |output: &Path| -> BTreeMap<String, Vec<u8>> {
            fs::read_dir(output)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    (
                        entry.file_name().to_string_lossy().into_owned(),
                        fs::read(entry.path()).unwrap(),
                    )
                })
                .collect()
        };
        assert_eq!(files(&outputs[0]), files(&outputs[1]));
        assert_eq!(files(&outputs[0]).len(), 6);
        for (target, _) in TARGETS {
            let extension = if is_windows(target) {
                ".zip"
            } else {
                ".tar.gz"
            };
            let entries = entries_of(&outputs[0].join(format!("russet-0.1.0-{target}{extension}")));
            for (name, entry) in payload(target) {
                let retained = if name == "README.md" || name == "INSTALL.md" {
                    format!("DEVELOPMENT-{name}")
                } else {
                    name
                };
                assert_eq!(entries[&retained], entry, "{target}: {retained}");
            }
            let metadata: Value = serde_json::from_slice(&entries["RELEASE.json"].0).unwrap();
            assert_eq!(metadata["source_commit"], SHA);
            assert_eq!(metadata["compatibility_version"], "3.0.0");
            assert_eq!(metadata["target"], target);
            assert_eq!(metadata["gates"]["development"]["id"], 1);
            assert!(String::from_utf8(entries["README.md"].0.clone())
                .unwrap()
                .starts_with("# Russet 0.1.0\n"));
        }
        let sums = fs::read_to_string(outputs[0].join("SHA256SUMS")).unwrap();
        assert_eq!(sums.lines().count(), 5);
        for line in sums.lines() {
            let (expected, name) = line.split_once("  ").unwrap();
            assert_eq!(digest(&fs::read(outputs[0].join(name)).unwrap()), expected);
        }
    }

    #[test]
    fn invalid_download_never_publishes_partial_output() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path();
        let api = FakeApi::new(base);
        api.raws
            .borrow_mut()
            .get_mut(&13)
            .unwrap()
            .extend(b"tampered");
        let output = base.join("output");
        let error = promote(&api, SHA, "0.1.0", 1, &output).unwrap_err();
        assert!(error.contains("digest mismatch"), "{error}");
        assert!(!output.exists());
        assert!(!fs::read_dir(base).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".russet-promotion-")));
    }

    #[test]
    fn stale_artifact_from_an_earlier_attempt_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let mut api = FakeApi::new(directory.path());
        api.artifacts[2]["created_at"] = json!("2025-12-31T23:59:00Z");
        let output = directory.path().join("out");
        assert!(promote(&api, SHA, "0.1.0", 1, &output).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn existing_output_is_never_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let api = FakeApi::new(directory.path());
        let output = directory.path().join("out");
        fs::create_dir(&output).unwrap();
        let error = promote(&api, SHA, "0.1.0", 1, &output).unwrap_err();
        assert_eq!(error, "Output directory must not already exist");
    }

    #[test]
    fn version_must_be_major_minor_patch() {
        let directory = tempfile::tempdir().unwrap();
        let api = FakeApi::new(directory.path());
        for version in ["1.0", "01.0.0", "v0.1.0", "0.1.0-beta", "0..1", "1.0.0.0"] {
            let error = promote(&api, SHA, version, 1, &directory.path().join("out")).unwrap_err();
            assert!(error.contains("MAJOR.MINOR.PATCH"), "{version}");
        }
        for version in ["0.1.0", "10.20.30", "1.01.0"] {
            assert!(is_version(version), "{version}");
        }
    }

    #[test]
    fn api_pagination_reads_all_pages() {
        struct Pages;
        impl Api for Pages {
            fn get(&self, endpoint: &str) -> Result<Value, String> {
                Ok(if endpoint.ends_with("page=1") {
                    json!({"jobs": (0..100).collect::<Vec<_>>()})
                } else {
                    json!({"jobs": [100]})
                })
            }
            fn raw(&self, _endpoint: &str) -> Result<Vec<u8>, String> {
                unreachable!()
            }
        }
        let pages = Pages.pages("endpoint", "jobs").unwrap();
        assert_eq!(pages, (0..=100).map(|n| json!(n)).collect::<Vec<_>>());
    }

    #[test]
    fn json_matches_python_sorted_ascii_layout() {
        let value = json!({"b": [1, {"z": null, "a": true}], "a": "café 🚀", "e": [], "d": {}});
        assert_eq!(
            String::from_utf8(json_bytes(&value)).unwrap(),
            "{\n  \"a\": \"caf\\u00e9 \\ud83d\\ude80\",\n  \"b\": [\n    1,\n    {\n      \"a\": true,\n      \"z\": null\n    }\n  ],\n  \"d\": {},\n  \"e\": []\n}\n"
        );
    }
}
