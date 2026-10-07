//! Native replacement for the `xar` operations Russet uses: reading and
//! extracting xar archives, the container format of flat packages.
//!
//! [`Archive::open`] is strict, because a package's signature covers only the
//! table of contents (TOC) checksum: the TOC is decompressed only after its
//! checksum matches, the TOC must contain exactly one checksum element, and
//! every heap range must lie inside the file without overlapping the checksum.
//! File data is verified against its archived and extracted checksums.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod archive;
#[cfg(unix)]
mod builder;
#[cfg(unix)]
mod digest;
#[cfg(all(test, unix))]
mod tests;

#[cfg(unix)]
pub use archive::{Archive, Data, Encoding, Entry, EntryKind, Signature};
#[cfg(unix)]
pub use builder::{Builder, Content};
#[cfg(unix)]
pub use digest::Algorithm;
