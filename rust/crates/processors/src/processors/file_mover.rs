//! `FileMover`: rename a file the way Python's `os.rename` does.
use super::Output;
use crate::{io, string, ExecutionFailure, Result};
use plist::Dictionary;

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let source = string(env, "source")?;
    let target = string(env, "target")?;
    io(python_rename(source, target))?;
    output(env, 1, format!("File {source} moved to {target}"));
    Ok(())
}

/// The standalone form, where a failed rename is an unexpected error rather
/// than a processor error, as it is in Python.
pub(crate) fn execute_standalone(
    env: &mut Dictionary,
    output: Output,
) -> std::result::Result<(), ExecutionFailure> {
    let source = string(env, "source")?;
    let target = string(env, "target")?;
    python_rename(source, target).map_err(|e| ExecutionFailure::unexpected(e.to_string()))?;
    output(env, 1, format!("File {source} moved to {target}"));
    Ok(())
}

#[cfg(not(windows))]
fn python_rename(source: &str, target: &str) -> std::io::Result<()> {
    std::fs::rename(source, target)
}

#[cfg(windows)]
fn python_rename(source: &str, target: &str) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    // Python os.rename uses MoveFileExW without MOVEFILE_REPLACE_EXISTING.
    // std::fs::rename replaces the destination, which changes Python behavior.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |path: &str| -> std::io::Result<Vec<u16>> {
        if path.contains('\0') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "embedded null character",
            ));
        }
        Ok(std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(Some(0))
            .collect())
    };
    let source = wide(source)?;
    let target = wide(target)?;
    // Both pointers refer to terminated UTF-16 buffers retained through this call.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 0) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
