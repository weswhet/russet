//! `FileCreator`: write text to a file, and optionally set its mode.
use super::Output;
use crate::{io, mode, string, Result};
use plist::Dictionary;
use std::{fs, path::Path};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let p = string(env, "file_path")?;
    io(write_python_text(p, string(env, "file_content")?))?;
    output(env, 1, format!("Created file at {p}"));
    if env.contains_key("file_mode") {
        mode(Path::new(p), string(env, "file_mode")?)?;
    }
    Ok(())
}

/// Write text the way Python's text mode does, with CRLF line endings on
/// Windows.
fn write_python_text(path: &str, content: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let content = content.replace('\n', "\r\n");
    fs::write(path, content)
}
