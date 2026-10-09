//! `DmgCreator`: create a disk image from a folder, natively or with
//! `hdiutil`.
use crate::{io, string, truth, Result};
use plist::{Dictionary, Value};
use std::{path::Path, process::Command};

fn mac() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err("Disk image operations are only supported on macOS; hdiutil is unavailable on this platform".into())
    }
}
fn number(value: &Value) -> Result<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Real(n) => Ok(n.to_string()),
        _ => Err("Expected numeric value".into()),
    }
}
pub(crate) fn execute(env: &Dictionary) -> Result<()> {
    use autopkg_platform::backend::{select, Backend, Tool};
    let backend = select(Tool::Hdiutil);
    if backend == Backend::Unsupported {
        mac()?;
    }
    let path = string(env, "dmg_path")?;
    if Path::new(path).exists() {
        io(std::fs::remove_file(path))?;
    }
    let format = string(env, "dmg_format")?;
    if ![
        "UDRW", "UDRO", "UDCO", "UDZO", "UDBZ", "UFBI", "UDTO", "UDxx", "UDSP", "UDSB", "ULFO",
        "ULMO",
    ]
    .contains(&format)
    {
        return Err(format!("dmg format '{format}' is invalid"));
    }
    let level = number(env.get("dmg_zlib_level").ok_or("Missing dmg_zlib_level")?)?
        .parse::<i64>()
        .map_err(|e| e.to_string())?;
    if !(1..=9).contains(&level) {
        return Err("dmg_zlib_level must be a value between 1 and 9.".into());
    }
    let filesystem = string(env, "dmg_filesystem")?;
    if ![
        "APFS",
        "Case-insensitive APFS",
        "Case-sensitive APFS",
        "Case-sensitive HFS+",
        "Case-sensitive Journaled HFS+",
        "ExFAT",
        "HFS+",
        "Journaled HFS+",
        "MS-DOS FAT12",
        "MS-DOS FAT16",
        "MS-DOS FAT32",
        "MS-DOS",
        "UDF",
    ]
    .contains(&filesystem)
    {
        return Err(format!("dmg filesystem '{filesystem}' is invalid"));
    }
    if backend == Backend::Native {
        let megabytes = if truth(env.get("dmg_megabytes")) {
            Some(
                number(&env["dmg_megabytes"])?
                    .parse::<u64>()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let written = native_create(
            string(env, "dmg_root")?,
            path,
            filesystem,
            format,
            level as u32,
            megabytes,
        )?;
        if written != filesystem {
            autopkg_platform::processor_output(
                0,
                format!(
                    "WARNING: {filesystem} disk images can't be created natively; created {written} instead"
                ),
            );
        }
        autopkg_platform::processor_output(
            1,
            format!("Created dmg from {} at {path}", string(env, "dmg_root")?),
        );
        return Ok(());
    }
    let mut command = Command::new("/usr/bin/hdiutil");
    command.args(["create", "-plist", "-fs", filesystem, "-format", format]);
    if format == "UDZO" {
        command.args(["-imagekey", &format!("zlib-level={level}")]);
    }
    if truth(env.get("dmg_megabytes")) {
        command
            .arg("-megabytes")
            .arg(number(&env["dmg_megabytes"])?);
    }
    command.args(["-srcfolder", string(env, "dmg_root")?, path]);
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        autopkg_platform::processor_output(
            1,
            format!("Created dmg from {} at {path}", string(env, "dmg_root")?),
        );
        Ok(())
    } else {
        Err(format!(
            "creation of {path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
#[cfg(unix)]
fn native_create(
    root: &str,
    path: &str,
    filesystem: &str,
    format: &str,
    zlib_level: u32,
    megabytes: Option<u64>,
) -> Result<&'static str> {
    russet_hdiutil::create(
        Path::new(root),
        Path::new(path),
        &russet_hdiutil::CreateOptions {
            filesystem,
            format,
            zlib_level,
            megabytes,
        },
    )
    .map_err(|e| format!("creation of {path} failed: {e}"))
}

#[cfg(not(unix))]
fn native_create(
    _: &str,
    _: &str,
    _: &str,
    _: &str,
    _: u32,
    _: Option<u64>,
) -> Result<&'static str> {
    Err("Native disk image creation requires macOS or Linux".into())
}
