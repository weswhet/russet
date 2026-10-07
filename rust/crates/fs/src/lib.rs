//! Safe extraction of untrusted archive and disk-image contents.
//!
//! Russet's native replacements for Apple's tools write files that come from
//! downloaded archives, packages, and disk images. [`TreeWriter`] confines
//! every write to one destination directory: paths are cleaned lexically,
//! every directory is opened relative to its parent without following
//! symlinks, files are created exclusively, and symlinks are created only
//! after everything else is written. [`Limits`] bounds the work an archive can
//! demand, and [`manifest`] describes a tree so tests can compare a native
//! result with the one Apple's tool produced.
#![forbid(unsafe_code)]

mod limits;
mod path;

pub use limits::Limits;
pub use path::clean_relative;

#[cfg(unix)]
mod manifest;
#[cfg(unix)]
mod writer;
#[cfg(unix)]
mod xattr_name;

#[cfg(unix)]
pub use manifest::{manifest, Entry, EntryKind};
#[cfg(unix)]
pub use writer::{SkippedXattr, TreeWriter};
#[cfg(unix)]
pub use xattr_name::{apple_xattr_name, host_xattr_name};

/// Builds the error used when untrusted input breaks a rule.
pub(crate) fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}
