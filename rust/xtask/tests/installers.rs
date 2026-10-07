//! Exercise the archive installers entirely inside temporary staging roots.
//!
//! The POSIX shell installer tests run on macOS and Linux. The PowerShell
//! installer tests run wherever `pwsh` is on PATH, which includes every
//! GitHub-hosted runner, and are skipped with a message otherwise.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

fn distribution() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../distribution")
}

/// A canonical temporary directory. On Windows the `\\?\` prefix is removed so
/// PowerShell and the installers see an ordinary drive path.
fn resolved(path: &Path) -> PathBuf {
    let path = fs::canonicalize(path).unwrap();
    #[cfg(windows)]
    if let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    path
}

fn write(path: &Path, data: &[u8], mode: u32) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
    set_mode(path, mode);
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

#[cfg(unix)]
fn mode_of(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn mode_of(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).unwrap();
}

/// A stored entry captured without following symbolic links or normalizing
/// modes.
#[derive(Debug, PartialEq)]
enum Entry {
    Link(u32, PathBuf),
    Directory(u32, BTreeMap<OsString, Entry>),
    File(u32, Vec<u8>),
}

fn snapshot(path: &Path) -> Option<Entry> {
    let metadata = fs::symlink_metadata(path).ok()?;
    let mode = mode_of(&metadata);
    Some(if metadata.file_type().is_symlink() {
        Entry::Link(mode, fs::read_link(path).unwrap())
    } else if metadata.is_dir() {
        Entry::Directory(
            mode,
            fs::read_dir(path)
                .unwrap()
                .map(|child| {
                    let child = child.unwrap();
                    (child.file_name(), snapshot(&child.path()).unwrap())
                })
                .collect(),
        )
    } else {
        Entry::File(mode, fs::read(path).unwrap())
    })
}

/// Find `name` on PATH, like `shutil.which`.
fn which(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_owned()]
    } else {
        vec![name.to_owned()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Run a command and fail the test if it takes longer than `seconds`.
fn output_with_timeout(command: &mut Command, seconds: u64) -> Output {
    use std::io::Read;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let stdout = thread::spawn(move || {
        let mut data = Vec::new();
        stdout.read_to_end(&mut data).unwrap();
        data
    });
    let stderr = thread::spawn(move || {
        let mut data = Vec::new();
        stderr.read_to_end(&mut data).unwrap();
        data
    });
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{command:?} did not finish within {seconds} seconds");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

#[cfg(unix)]
mod shell {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    type Environment = Vec<(String, OsString)>;
    const NONE: &[(String, OsString)] = &[];

    struct Staging {
        temp: PathBuf,
        platform: &'static str,
        root: PathBuf,
        source: PathBuf,
        destination: PathBuf,
        command: PathBuf,
        daemons: [PathBuf; 2],
        managed: Vec<PathBuf>,
        unrelated: PathBuf,
    }

    impl Staging {
        fn new(temp: &Path, platform: &'static str) -> Self {
            let root = temp.join("staged root");
            fs::create_dir(&root).unwrap();
            let source = temp.join("archive source");
            fs::create_dir(&source).unwrap();
            fs::copy(distribution().join("install.sh"), source.join("install.sh")).unwrap();
            let destination = root.join(if platform == "Darwin" {
                "Library/AutoPkg"
            } else {
                "usr/local/lib/autopkg"
            });
            let command = root.join("usr/local/bin/autopkg");
            let daemons = ["autopkgserver", "autopkginstalld"].map(|name| {
                root.join(format!(
                    "Library/LaunchDaemons/com.github.autopkg.{name}.plist"
                ))
            });
            let mut managed = vec![destination.clone(), command.clone()];
            if platform == "Darwin" {
                managed.extend(daemons.iter().cloned());
            }
            let unrelated = root.join("Library/Preferences/com.github.autopkg.plist");
            let stage = Self {
                temp: temp.to_owned(),
                platform,
                root,
                source,
                destination,
                command,
                daemons,
                managed,
                unrelated,
            };
            stage.payload(b"release one\n");
            write(&stage.unrelated, b"existing preferences\x00\xff", 0o600);
            stage
        }

        fn payload(&self, content: &[u8]) {
            for name in ["autopkg-rs", "autopkgserver-rs", "autopkginstalld-rs"] {
                let data = [content, name.as_bytes()].concat();
                write(&self.source.join("bin").join(name), &data, 0o755);
            }
            for name in ["autopkgserver", "autopkginstalld"] {
                let data = [b"new ".as_slice(), name.as_bytes()].concat();
                write(
                    &self.source.join("launchd").join(format!("{name}.plist")),
                    &data,
                    0o644,
                );
            }
        }

        fn legacy(&self) {
            write(
                &self.destination.join("autopkg"),
                b"legacy executable\x00",
                0o751,
            );
            write(
                &self.destination.join("nested/preferences"),
                b"preserve exactly",
                0o640,
            );
            symlink(
                Path::new("preferences"),
                &self.destination.join("nested/relative-link"),
            );
            symlink(Path::new("absent"), &self.destination.join("broken-link"));
            set_mode(&self.destination, 0o750);
            fs::create_dir_all(self.command.parent().unwrap()).unwrap();
            symlink(Path::new("../legacy/autopkg"), &self.command);
            if self.platform == "Darwin" {
                write(&self.daemons[0], b"legacy daemon", 0o600);
                symlink(Path::new("old-installation.plist"), &self.daemons[1]);
            }
        }

        fn state(&self) -> Vec<Option<Entry>> {
            self.managed
                .iter()
                .chain([&self.unrelated])
                .map(|path| snapshot(path))
                .collect()
        }

        fn run(&self, action: &str, environment: &[(String, OsString)]) -> Output {
            let mut command = Command::new("/bin/sh");
            command
                .arg(self.source.join("install.sh"))
                .args([action, "--root"])
                .arg(&self.root)
                .args(["--platform", self.platform]);
            for (name, value) in environment {
                command.env(name, value);
            }
            output_with_timeout(&mut command, 120)
        }

        /// Put a shim for `command` first on PATH. The first time any of its
        /// arguments contains `pattern`, it fails (or, with `signal`, sends
        /// SIGTERM to the installer and reports success); otherwise it runs
        /// the real command.
        fn failure(&self, command: &str, pattern: &str, signal: bool) -> Environment {
            let shim = self.source.join("failure-shim");
            fs::create_dir_all(&shim).unwrap();
            let actual = which(command).unwrap();
            let script = format!(
                r#"#!/bin/sh
if [ ! -e "$AUTOPKG_FAILURE_ONCE" ]; then
    for argument in "$@"; do
        case $argument in
            *"$AUTOPKG_FAILURE_MATCH"*)
                printf 'injected\n' > "$AUTOPKG_FAILURE_ONCE"
                printf 'injected failure\n' >&2
                if [ -n "${{AUTOPKG_FAILURE_SIGNAL:-}}" ]; then
                    kill -TERM "$PPID"
                    exit 0
                fi
                exit 73
                ;;
        esac
    done
fi
exec '{}' "$@"
"#,
                actual.display()
            );
            write(&shim.join(command), script.as_bytes(), 0o755);
            let path = std::env::join_paths(std::iter::once(shim.clone()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap();
            let mut environment = vec![
                ("PATH".to_owned(), path),
                (
                    "AUTOPKG_FAILURE_ONCE".to_owned(),
                    shim.join("triggered").into(),
                ),
                ("AUTOPKG_FAILURE_MATCH".to_owned(), pattern.into()),
            ];
            if signal {
                environment.push(("AUTOPKG_FAILURE_SIGNAL".to_owned(), "1".into()));
            }
            environment
        }
    }

    fn assert_success(output: &Output) {
        assert!(
            output.status.success(),
            "{}{}",
            text(&output.stdout),
            text(&output.stderr)
        );
    }

    fn temp() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = resolved(directory.path());
        (directory, path)
    }

    const PLATFORMS: [&str; 2] = ["Linux", "Darwin"];

    #[test]
    fn fresh_install_and_rollback() {
        for platform in PLATFORMS {
            let (_directory, temp) = temp();
            let stage = Staging::new(&temp, platform);
            let before = stage.state();
            assert_success(&stage.run("install", NONE));
            let installed = stage.destination.join("autopkg");
            assert_eq!(
                fs::read(&installed).unwrap(),
                fs::read(stage.source.join("bin/autopkg-rs")).unwrap()
            );
            assert_eq!(mode_of(&fs::metadata(&installed).unwrap()), 0o755);
            assert_eq!(
                fs::read_link(&stage.command).unwrap(),
                fs::canonicalize(&installed).unwrap()
            );
            if platform == "Darwin" {
                for daemon in &stage.daemons {
                    assert_eq!(mode_of(&fs::metadata(daemon).unwrap()), 0o644);
                }
                assert!(stage
                    .destination
                    .join("autopkgserver/autopkginstalld")
                    .is_file());
            }
            assert_success(&stage.run("rollback", NONE));
            assert_eq!(stage.state(), before, "{platform}");
        }
    }

    #[test]
    fn two_upgrades_restore_exact_legacy_entries() {
        for platform in PLATFORMS {
            let (_directory, temp) = temp();
            let stage = Staging::new(&temp, platform);
            stage.legacy();
            let original = stage.state();
            let inode = fs::metadata(stage.destination.join("autopkg"))
                .unwrap()
                .ino();
            assert_success(&stage.run("install", NONE));
            let first = stage.state();
            stage.payload(b"release two\n");
            assert_success(&stage.run("install", NONE));
            assert_success(&stage.run("rollback", NONE));
            assert_eq!(stage.state(), first, "{platform}");
            assert_success(&stage.run("rollback", NONE));
            assert_eq!(stage.state(), original, "{platform}");
            assert_eq!(
                fs::metadata(stage.destination.join("autopkg"))
                    .unwrap()
                    .ino(),
                inode
            );
        }
    }

    #[test]
    fn install_failure_restores_previous_generation() {
        for platform in PLATFORMS {
            let mut cases = vec![
                ("cp", "autopkg-rs"),
                ("mv", "previous-command"),
                ("mv", "candidate"),
                ("ln", "autopkg"),
            ];
            if platform == "Darwin" {
                cases.extend([
                    ("mv", "next-packaging.plist"),
                    ("mv", "next-installation.plist"),
                ]);
            }
            for (command, pattern) in cases {
                let (_directory, temp) = temp();
                let stage = Staging::new(&temp, platform);
                stage.legacy();
                let before = stage.state();
                let result = stage.run("install", &stage.failure(command, pattern, false));
                let context = format!("{platform} {command} {pattern}");
                assert!(!result.status.success(), "{context}");
                assert!(
                    text(&result.stderr).contains("injected failure"),
                    "{context}"
                );
                assert_eq!(stage.state(), before, "{context}");
            }
        }
    }

    #[test]
    fn rollback_failure_preserves_current_generation() {
        for platform in PLATFORMS {
            let mut cases = vec![
                "/retired/installation",
                "/retired/command",
                "previous-installation",
                "previous-command",
            ];
            if platform == "Darwin" {
                cases.extend([
                    "/retired/packaging.plist",
                    "/retired/installation.plist",
                    "previous-packaging.plist",
                ]);
            }
            for pattern in cases {
                let (_directory, temp) = temp();
                let stage = Staging::new(&temp, platform);
                stage.legacy();
                let original = stage.state();
                assert_success(&stage.run("install", NONE));
                let before = stage.state();
                let result = stage.run("rollback", &stage.failure("mv", pattern, false));
                let context = format!("{platform} {pattern}");
                assert!(!result.status.success(), "{context}");
                assert!(
                    text(&result.stderr).contains("injected failure"),
                    "{context}"
                );
                assert_eq!(stage.state(), before, "{context}");
                assert_success(&stage.run("rollback", NONE));
                assert_eq!(stage.state(), original, "{context}");
            }
        }
    }

    #[test]
    fn tampered_rollback_record_rejects_without_mutation() {
        for platform in PLATFORMS {
            for tamper in ["outside", "symlink", "uncommitted"] {
                let (_directory, temp) = temp();
                let stage = Staging::new(&temp, platform);
                stage.legacy();
                assert_success(&stage.run("install", NONE));
                let marker = stage.destination.join(".autopkg-rust-rollback");
                match tamper {
                    "outside" => {
                        fs::write(
                            &marker,
                            stage.temp.join("generation.outside").to_str().unwrap(),
                        )
                        .unwrap();
                    }
                    "symlink" => {
                        let saved = stage.temp.join("marker");
                        fs::rename(&marker, &saved).unwrap();
                        symlink(&saved, &marker);
                    }
                    _ => {
                        let generation = fs::read_to_string(&marker).unwrap();
                        fs::remove_file(Path::new(generation.trim()).join("committed")).unwrap();
                    }
                }
                let before = snapshot(&stage.root);
                let result = stage.run("rollback", NONE);
                assert!(!result.status.success(), "{platform} {tamper}");
                assert_eq!(snapshot(&stage.root), before, "{platform} {tamper}");
            }
        }
    }

    #[test]
    fn interrupted_install_reports_failure_and_restores_previous_entries() {
        for platform in PLATFORMS {
            for action in ["install", "rollback"] {
                let (_directory, temp) = temp();
                let stage = Staging::new(&temp, platform);
                stage.legacy();
                if action == "rollback" {
                    assert_success(&stage.run("install", NONE));
                }
                let before = stage.state();
                let (command, pattern) = if action == "install" {
                    ("ln", "autopkg")
                } else {
                    ("mv", "previous-command")
                };
                let result = stage.run(action, &stage.failure(command, pattern, true));
                let context = format!("{platform} {action}: SIGTERM must not report success");
                assert!(!result.status.success(), "{context}");
                assert_eq!(stage.state(), before, "{context}");
            }
        }
    }

    #[test]
    fn legacy_destination_symlink_restores_without_touching_target() {
        for platform in PLATFORMS {
            let (_directory, temp) = temp();
            let stage = Staging::new(&temp, platform);
            let external = temp.join("legacy target");
            write(&external.join("autopkg"), b"original linked release", 0o751);
            fs::create_dir_all(stage.destination.parent().unwrap()).unwrap();
            symlink(&external, &stage.destination);
            let before = stage.state();
            let target = snapshot(&external);
            assert_success(&stage.run("install", NONE));
            assert_eq!(snapshot(&external), target, "{platform}");
            assert_success(&stage.run("rollback", NONE));
            assert_eq!(stage.state(), before, "{platform}");
            assert_eq!(snapshot(&external), target, "{platform}");
        }
    }

    #[test]
    fn missing_payload_and_parent_symlinks_reject_before_mutation() {
        for platform in PLATFORMS {
            for failure in ["missing", "symlink"] {
                let (_directory, temp) = temp();
                let stage = Staging::new(&temp, platform);
                if failure == "missing" {
                    fs::remove_file(stage.source.join("bin/autopkg-rs")).unwrap();
                } else {
                    let external = temp.join("external");
                    fs::create_dir(&external).unwrap();
                    let parent = stage.command.parent().unwrap();
                    fs::create_dir_all(parent.parent().unwrap()).unwrap();
                    symlink(&external, parent);
                }
                let before = snapshot(&stage.root);
                let result = stage.run("install", NONE);
                assert!(!result.status.success(), "{platform} {failure}");
                assert_eq!(snapshot(&stage.root), before, "{platform} {failure}");
            }
        }
    }
}

mod powershell {
    use super::*;

    fn pwsh() -> Option<PathBuf> {
        let found = which("pwsh");
        if found.is_none() {
            eprintln!("skipping PowerShell installer test: pwsh is not on PATH");
        }
        found
    }

    struct Fixture {
        _directory: tempfile::TempDir,
        temp: PathBuf,
        pwsh: PathBuf,
        source: PathBuf,
        destination: PathBuf,
    }

    impl Fixture {
        fn new(pwsh: PathBuf) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let temp = resolved(directory.path());
            let source = temp.join("source");
            fs::create_dir(&source).unwrap();
            fs::copy(
                distribution().join("install.ps1"),
                source.join("install.ps1"),
            )
            .unwrap();
            let destination = temp.join("installation");
            Self {
                _directory: directory,
                temp,
                pwsh,
                source,
                destination,
            }
        }

        fn run(&self, action: &str) -> Output {
            output_with_timeout(
                Command::new(&self.pwsh)
                    .args(["-NoProfile", "-File"])
                    .arg(self.source.join("install.ps1"))
                    .args([action, "-Destination"])
                    .arg(&self.destination),
                60,
            )
        }

        fn run_ok(&self, action: &str) {
            let output = self.run(action);
            assert!(
                output.status.success(),
                "{action}: {}{}",
                text(&output.stdout),
                text(&output.stderr)
            );
        }

        /// Run the installer through `wrapper`, a script that redefines
        /// Move-Item, with the given environment.
        fn wrapper(&self, name: &str, script: &str, action: &str) -> Command {
            let path = self.temp.join(name);
            fs::write(&path, script).unwrap();
            let mut command = Command::new(&self.pwsh);
            command
                .args(["-NoProfile", "-File"])
                .arg(&path)
                .env("AUTOPKG_SCRIPT", self.source.join("install.ps1"))
                .env("AUTOPKG_ACTION", action)
                .env("AUTOPKG_DESTINATION", &self.destination);
            command
        }
    }

    const INJECT: &str = r#"$ErrorActionPreference = 'Stop'
function global:Move-Item {
    param([string]$LiteralPath, [string]$Destination)
    if (-not (Test-Path -LiteralPath $env:AUTOPKG_FAILURE_ONCE) -and
        $LiteralPath.Contains($env:AUTOPKG_FAILURE_MATCH)) {
        Set-Content -LiteralPath $env:AUTOPKG_FAILURE_ONCE -Value 'injected'
        throw 'injected failure'
    }
    Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination
}
& $env:AUTOPKG_SCRIPT $env:AUTOPKG_ACTION -Destination $env:AUTOPKG_DESTINATION
"#;

    const PAUSE: &str = r#"$ErrorActionPreference = 'Stop'
function global:Move-Item {
    param([string]$LiteralPath, [string]$Destination)
    Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination
    $MatchesSource = if ($env:AUTOPKG_PAUSE_SOURCE -eq 'destination') {
        $LiteralPath -eq $env:AUTOPKG_DESTINATION
    } else { (Split-Path -Leaf $LiteralPath) -eq $env:AUTOPKG_PAUSE_SOURCE }
    if ($MatchesSource) {
        Set-Content -LiteralPath $env:AUTOPKG_PAUSE_MARKER -Value 'moved'
        Start-Sleep -Seconds 60
    }
}
& $env:AUTOPKG_SCRIPT $env:AUTOPKG_ACTION -Destination $env:AUTOPKG_DESTINATION
"#;

    #[test]
    fn failed_install_and_rollback_preserve_previous_state() {
        let Some(pwsh) = pwsh() else { return };
        for (action, pattern) in [("install", "candidate"), ("rollback", "previous")] {
            let fixture = Fixture::new(pwsh.clone());
            write(&fixture.destination.join("autopkg.exe"), b"legacy", 0o644);
            write(&fixture.source.join("bin/autopkg-rs.exe"), b"native", 0o644);
            if action == "rollback" {
                fixture.run_ok("install");
            }
            let before = snapshot(&fixture.destination);
            let mut command = fixture.wrapper("inject.ps1", INJECT, action);
            command
                .env("AUTOPKG_FAILURE_MATCH", pattern)
                .env("AUTOPKG_FAILURE_ONCE", fixture.temp.join("triggered"));
            let result = output_with_timeout(&mut command, 60);
            assert!(!result.status.success(), "{action}");
            assert!(
                text(&result.stderr).contains("injected failure"),
                "{action}"
            );
            assert_eq!(snapshot(&fixture.destination), before, "{action}");
        }
    }

    #[test]
    fn forced_termination_recovers_each_directory_move() {
        let Some(pwsh) = pwsh() else { return };
        for (action, pause_source) in [
            ("install", "destination"),
            ("install", "candidate"),
            ("rollback", "destination"),
            ("rollback", "previous"),
        ] {
            let context = format!("{action} {pause_source}");
            let fixture = Fixture::new(pwsh.clone());
            write(&fixture.destination.join("autopkg.exe"), b"legacy", 0o644);
            write(
                &fixture.destination.join("preferences.dat"),
                b"preserve me",
                0o644,
            );
            let original = snapshot(&fixture.destination);
            write(&fixture.source.join("bin/autopkg-rs.exe"), b"native", 0o644);
            if action == "rollback" {
                fixture.run_ok("install");
            }
            let marker = fixture.temp.join("paused");
            let mut child = fixture
                .wrapper("pause.ps1", PAUSE, action)
                .env("AUTOPKG_PAUSE_SOURCE", pause_source)
                .env("AUTOPKG_PAUSE_MARKER", &marker)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            while !marker.exists()
                && child.try_wait().unwrap().is_none()
                && Instant::now() < deadline
            {
                thread::sleep(Duration::from_millis(20));
            }
            let reached = marker.exists();
            // No exception or finally cleanup can run after a forced kill.
            let _ = child.kill();
            child.wait().unwrap();
            assert!(
                reached,
                "{context}: installer did not reach the injected move boundary"
            );
            let history = PathBuf::from(format!("{}-rollbacks", fixture.destination.display()));
            let journal = history.join("pending.json");
            assert!(journal.is_file(), "{context}");
            let blocked = fixture.run("install");
            assert!(!blocked.status.success(), "{context}");
            assert!(
                text(&blocked.stderr).contains("Interrupted transaction"),
                "{context}: {}",
                text(&blocked.stderr)
            );
            fixture.run_ok("recover");
            assert_eq!(snapshot(&fixture.destination), original, "{context}");
            assert!(!journal.exists(), "{context}");
            let retained = fs::read_dir(&history)
                .unwrap()
                .map(|entry| entry.unwrap())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("generation.")
                })
                .flat_map(|generation| fs::read_dir(generation.path()).unwrap())
                .map(|entry| entry.unwrap().path().join("autopkg.exe"))
                .filter(|path| fs::read(path).is_ok_and(|data| data == b"native"))
                .count();
            assert_eq!(
                retained, 1,
                "{context}: the interrupted native generation must remain"
            );
            fixture.run_ok("recover");
        }
    }

    #[test]
    fn recovery_rejects_external_generation_and_reparse_points() {
        let Some(pwsh) = pwsh() else { return };
        for attack in ["external", "symlink"] {
            let fixture = Fixture::new(pwsh.clone());
            let history = PathBuf::from(format!("{}-rollbacks", fixture.destination.display()));
            fs::create_dir(&history).unwrap();
            let outside = fixture.temp.join("outside");
            write(&outside.join("keep"), b"untouched", 0o644);
            let mut generation = history.join(format!("generation.{}", "a".repeat(32)));
            fs::create_dir(&generation).unwrap();
            if attack == "external" {
                generation = outside.clone();
            } else {
                symlink(&outside, &generation.join("previous"));
            }
            let pending = format!(
                r#"{{"version": 1, "action": "install", "generation": {}}}"#,
                serde_json::to_string(generation.to_str().unwrap()).unwrap()
            );
            fs::write(history.join("pending.json"), pending).unwrap();
            let before = snapshot(&outside);
            let result = output_with_timeout(
                Command::new(&fixture.pwsh)
                    .args(["-NoProfile", "-File"])
                    .arg(distribution().join("install.ps1"))
                    .args(["recover", "-Destination"])
                    .arg(&fixture.destination),
                60,
            );
            assert!(!result.status.success(), "{attack}");
            assert_eq!(snapshot(&outside), before, "{attack}");
            assert!(!fixture.destination.exists(), "{attack}");
            assert!(history.join("pending.json").exists(), "{attack}");
        }
    }

    #[test]
    fn install_upgrade_and_two_rollbacks() {
        let Some(pwsh) = pwsh() else { return };
        let fixture = Fixture::new(pwsh);
        write(
            &fixture.destination.join("autopkg.exe"),
            b"legacy\x00",
            0o644,
        );
        write(
            &fixture.destination.join("preferences.dat"),
            b"existing preferences",
            0o644,
        );
        let original = snapshot(&fixture.destination);
        write(
            &fixture.source.join("bin/autopkg-rs.exe"),
            b"release one",
            0o644,
        );
        fixture.run_ok("install");
        let first = snapshot(&fixture.destination);
        write(
            &fixture.source.join("bin/autopkg-rs.exe"),
            b"release two",
            0o644,
        );
        fixture.run_ok("install");
        assert_eq!(
            fs::read(fixture.destination.join("autopkg.exe")).unwrap(),
            b"release two"
        );
        fixture.run_ok("rollback");
        assert_eq!(snapshot(&fixture.destination), first);
        fixture.run_ok("rollback");
        assert_eq!(snapshot(&fixture.destination), original);
    }
}
