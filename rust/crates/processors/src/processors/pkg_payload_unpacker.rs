//! `PkgPayloadUnpacker`: unpack a package's Payload into a folder.
//!
//! Inputs and outputs: run `russet processor-info PkgPayloadUnpacker`, or see
//! `PkgPayloadUnpacker` in `compatibility/reference.json`.
use crate::package::{native, run};
use crate::{io, macos_path, remove, string, truth, Result};
use autopkg_platform::backend::{select, Backend, Tool};
use autopkg_platform::processor_output as output;
use plist::Dictionary;
use std::{fs, path::Path};

fn prepare_destination(env: &Dictionary) -> Result<()> {
    let path = Path::new(string(env, "destination_path")?);
    if !path.exists() {
        io(fs::create_dir_all(path))?;
    } else if truth(env.get("purge_destination")) {
        for entry in io(fs::read_dir(path))? {
            remove(&io(entry)?.path())?;
        }
    }
    Ok(())
}
pub(crate) fn execute(env: &Dictionary) -> Result<()> {
    let backend = select(Tool::Ditto);
    if backend == Backend::Unsupported {
        return Err(
            "Package payload extraction is only supported on macOS and Linux; ditto and aa are unavailable"
                .into(),
        );
    }
    prepare_destination(env)?;
    let source = &macos_path(string(env, "pkg_payload_path")?);
    let destination = string(env, "destination_path")?;
    if backend == Backend::Native {
        native::extract_payload(source, destination)?;
    } else {
        match run("/usr/bin/ditto", &["-x", "-z", source, destination]) {
            Ok(()) => Ok(()),
            Err(ditto_error) if select(Tool::Aa) == Backend::Native => {
                native::extract_apple_archive(source, destination)
                    .map_err(|error| format!("{ditto_error}; {error}"))
            }
            Err(ditto_error) if Path::new("/usr/bin/aa").exists() => {
                run("/usr/bin/aa", &["extract", "-i", source, "-d", destination])
                    .map_err(|error| format!("{ditto_error}; {error}"))
            }
            Err(error) => Err(error),
        }?;
    }
    output(1, format!("Unpacked {source} to {destination}"));
    Ok(())
}
