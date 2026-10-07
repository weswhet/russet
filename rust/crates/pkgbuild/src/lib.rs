//! Native replacement for the `pkgbuild` operations Russet uses:
//! [`analyze`] (`pkgbuild --analyze`) and [`build`] (a component package
//! from a root folder).
//!
//! Ownership and modes come from the caller as a list of [`Node`]s rather
//! than from the disk, so a package can record `root:wheel` files without
//! running as root. The payload is a gzip-compressed odc cpio archive, the
//! BOM lists the same paths, and `PackageInfo` follows `pkgbuild`'s layout.
//! See `compatibility/apple-tools/pkgbuild.md`.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod analyze;
#[cfg(unix)]
mod build;

#[cfg(unix)]
pub use analyze::{analyze, Component};
#[cfg(unix)]
pub use build::{build, collect, Node, NodeKind, Options};

#[cfg(all(test, unix))]
mod tests;
