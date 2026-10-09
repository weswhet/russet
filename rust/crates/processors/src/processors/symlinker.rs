//! `Symlinker`: create a symbolic link, replacing an existing file when
//! `overwrite` is set.
use super::Output;
use crate::{io, string, symlink, truth, Result};
use plist::Dictionary;
use std::{fs, path::Path};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let source = Path::new(string(env, "source_path")?);
    let dest = Path::new(string(env, "destination_path")?);
    if dest.exists() && truth(env.get("overwrite")) {
        io(fs::remove_file(dest))?;
    }
    symlink(source, dest)?;
    output(
        env,
        1,
        format!("Symlinked {} to {}", source.display(), dest.display()),
    );
    Ok(())
}
