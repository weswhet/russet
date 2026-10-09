//! `Copier`: copy a file or folder, found with a glob, to a destination.
//!
//! Inputs and outputs: run `russet processor-info Copier`, or see
//! `Copier` in `compatibility/reference.json`.
use super::Output;
use crate::{copy_tree, io, matches, remove, string, truth, Result};
use plist::Dictionary;
use std::{fs, path::Path};

pub(crate) fn execute(env: &mut Dictionary, output: Output) -> Result<()> {
    let pattern = string(env, "source_path")?;
    let paths = matches(pattern)?;
    let source = paths
        .first()
        .ok_or("Error processing source_path with glob")?;
    if paths.len() > 1 {
        output(
            env,
            1,
            format!("WARNING: Multiple paths match 'source_path' glob '{pattern}':"),
        );
        for path in &paths {
            output(env, 1, format!("  - {}", path.display()));
        }
    }
    if pattern.contains(['*', '?', '[', ']', '!']) {
        output(
            env,
            1,
            format!(
                "Using path '{}' matched from globbed '{pattern}'.",
                source.display()
            ),
        );
    }
    let dest = Path::new(string(env, "destination_path")?);
    if dest.exists() && truth(env.get("overwrite")) {
        remove(dest)?;
    }
    if source.is_dir() {
        copy_tree(source, dest)?;
        output(
            env,
            1,
            format!("Copied {} to {}", source.display(), dest.display()),
        );
        Ok(())
    } else {
        let target = if dest.is_dir() {
            dest.join(source.file_name().ok_or("Source has no filename")?)
        } else {
            dest.to_path_buf()
        };
        // copyfile does not transfer permissions when the destination is a file.
        let permissions = fs::metadata(&target).ok().map(|m| m.permissions());
        if source.canonicalize().ok() == target.canonicalize().ok() && target.exists() {
            return Err("Source and destination are the same file".into());
        }
        let bytes = io(fs::read(source))?;
        io(fs::write(&target, bytes))?;
        if dest.is_dir() {
            io(fs::set_permissions(
                &target,
                io(fs::metadata(source))?.permissions(),
            ))?;
        } else if let Some(p) = permissions {
            io(fs::set_permissions(&target, p))?;
        }
        output(
            env,
            1,
            format!("Copied {} to {}", source.display(), dest.display()),
        );
        Ok(())
    }
}
