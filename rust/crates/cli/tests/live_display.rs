//! `run` in a terminal draws the live display; `--plain`, `-v`, and a dumb
//! terminal keep the plain lines. Each run gets its own pseudo-terminal.
#![cfg(unix)]
use plist::{Dictionary, Value};
use std::{
    fs::{self, File},
    io::Read,
    os::fd::{FromRawFd, OwnedFd},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    prefs: PathBuf,
}

fn step(processor: &str, arguments: Dictionary) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        ("Processor", Value::from(processor)),
        ("Arguments", Value::Dictionary(arguments)),
    ]))
}

/// `Alpha` writes a file and deletes it; `Broken` writes into a folder
/// that doesn't exist. Neither needs the network.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let prefs = root.join("prefs.plist");
    Value::Dictionary(Dictionary::from_iter([
        (
            "CACHE_DIR",
            Value::from(root.join("cache").to_string_lossy().into_owned()),
        ),
        (
            "RECIPE_MAP_PATH",
            Value::from(root.join("map.json").to_string_lossy().into_owned()),
        ),
        (
            "RECIPE_SEARCH_DIRS",
            Value::Array(vec![root
                .join("recipes")
                .to_string_lossy()
                .into_owned()
                .into()]),
        ),
        ("RECIPE_OVERRIDE_DIRS", Value::Array(vec![])),
    ]))
    .to_file_xml(&prefs)
    .unwrap();
    fs::create_dir(root.join("recipes")).unwrap();
    let file = |path: &str| {
        Dictionary::from_iter([
            ("file_path", Value::from(path)),
            ("file_content", Value::from("hello")),
        ])
    };
    let recipes = [
        (
            "Alpha",
            vec![
                step("FileCreator", file("%RECIPE_CACHE_DIR%/alpha.txt")),
                step(
                    "PathDeleter",
                    Dictionary::from_iter([(
                        "path_list",
                        Value::Array(vec!["%RECIPE_CACHE_DIR%/alpha.txt".into()]),
                    )]),
                ),
            ],
        ),
        (
            "Broken",
            vec![step(
                "FileCreator",
                file("%RECIPE_CACHE_DIR%/missing/broken.txt"),
            )],
        ),
    ];
    for (name, process) in recipes {
        Value::Dictionary(Dictionary::from_iter([
            ("Identifier", Value::from(format!("org.test.{name}"))),
            ("Input", Value::Dictionary(Dictionary::new())),
            ("Process", Value::Array(process)),
        ]))
        .to_file_xml(root.join("recipes").join(format!("{name}.recipe")))
        .unwrap();
    }
    let fixture = Fixture {
        _temp: temp,
        root,
        prefs,
    };
    let map = fixture
        .command("generate-recipe-map", &[], "dumb")
        .output()
        .unwrap();
    assert!(map.status.success());
    fixture
}

impl Fixture {
    fn command(&self, verb: &str, args: &[&str], term: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_russet"));
        for (key, _) in std::env::vars_os() {
            let key_text = key.to_string_lossy();
            if key_text.starts_with("AUTOPKG_") || matches!(&*key_text, "NO_COLOR" | "CI") {
                command.env_remove(key);
            }
        }
        command
            .env("HOME", &self.root)
            .env("TMPDIR", &self.root)
            .env("AUTOPKG_RS_PREFERENCES_FILE", &self.prefs)
            .env("TERM", term)
            .arg(verb)
            .args(["--prefs", self.prefs.to_str().unwrap()])
            .args(args);
        command
    }

    /// Runs with stdout and stderr on a new 100×40 terminal, and returns
    /// what it showed and the exit status.
    fn run_in_terminal(&self, args: &[&str], term: &str) -> (String, i32) {
        let (mut controller, device) = pty();
        let mut command = self.command("run", args, term);
        command
            .stdin(Stdio::null())
            .stdout(device.try_clone().unwrap())
            .stderr(device);
        let mut child = command.spawn().unwrap();
        drop(command);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut bytes = vec![];
            let mut buffer = [0; 4096];
            // A closed terminal reads as EOF on macOS and EIO on Linux.
            while let Ok(read @ 1..) = controller.read(&mut buffer) {
                bytes.extend_from_slice(&buffer[..read]);
            }
            let _ = sender.send(bytes);
        });
        let status = child.wait().unwrap();
        let bytes = receiver
            .recv_timeout(Duration::from_secs(30))
            .expect("the terminal closed");
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        (text, status.code().unwrap())
    }
}

/// A new terminal: the controlling side, and the side the child writes to.
fn pty() -> (File, File) {
    let (mut controller, mut device) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 40,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty fills both descriptors, which are then owned here.
    unsafe {
        assert_eq!(
            libc::openpty(
                &mut controller,
                &mut device,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            ),
            0
        );
        for fd in [controller, device] {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        (
            File::from(OwnedFd::from_raw_fd(controller)),
            File::from(OwnedFd::from_raw_fd(device)),
        )
    }
}

/// Removes escape sequences, then keeps what each line ends with after
/// its last carriage return.
fn strip(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            result.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    result
        .lines()
        .map(|line| line.rsplit('\r').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sorted_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    lines.sort();
    lines
}

#[test]
fn a_terminal_gets_the_live_display() {
    let fixture = fixture();
    let (raw, status) =
        fixture.run_in_terminal(&["Alpha", "Broken", "--jobs", "2"], "xterm-256color");
    assert_eq!(status, 70, "{raw}");
    assert!(raw.contains("\x1b[?2026h"), "{raw}");
    let shown = strip(&raw);
    assert!(shown.lines().any(|l| l.starts_with("✔ Alpha ")), "{shown}");
    assert!(shown.lines().any(|l| l.starts_with("✘ Broken ")), "{shown}");
    assert!(shown.contains("  ✔ FileCreator"), "{shown}");
    assert!(shown.contains("  ✔ PathDeleter"), "{shown}");
    assert!(shown.contains("  ✘ FileCreator"), "{shown}");
    assert!(shown.contains("The following recipes failed:"), "{shown}");
    assert!(!shown.contains("Processing "), "{shown}");
    assert!(!shown.contains("[Alpha"), "{shown}");
}

#[test]
fn plain_output_never_draws() {
    let fixture = fixture();
    let piped = fixture
        .command("run", &["Alpha", "Broken"], "xterm-256color")
        .output()
        .unwrap();
    let mut expected = String::from_utf8_lossy(&piped.stdout).into_owned();
    expected.push_str(&String::from_utf8_lossy(&piped.stderr));
    assert!(expected.contains("Processing Alpha..."), "{expected}");
    for (args, term) in [(vec!["--plain"], "xterm-256color"), (vec![], "dumb")] {
        let mut args = args;
        args.extend(["Alpha", "Broken"]);
        let (shown, status) = fixture.run_in_terminal(&args, term);
        assert_eq!(status, 70, "{shown}");
        assert!(!shown.contains('\x1b'), "{args:?} {term}: {shown:?}");
        assert_eq!(
            sorted_lines(&shown),
            sorted_lines(&expected),
            "{args:?} {term}"
        );
    }
    let (shown, _) = fixture.run_in_terminal(&["-v", "Alpha"], "xterm-256color");
    assert!(!shown.contains('\x1b'), "{shown:?}");
    assert!(shown.contains("Processing Alpha..."), "{shown}");
}
