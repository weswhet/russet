//! Native replacement for `mkbom` and `lsbom`: writing and reading Apple
//! bill-of-materials (BOM) files, which list a package payload's paths,
//! modes, owners, sizes, and checksums.
//!
//! The layout follows what `mkbom` writes on macOS: a `BOMStore` header, a
//! block table, and the variables `BomInfo`, `Paths`, `HLIndex`, `VIndex`,
//! and `Size64`. Each path has a record block, a name block keyed by its
//! parent's path ID, and an index block, listed in path-ID order (a sorted
//! depth-first walk) in a B+ tree of 4 KiB nodes. Checksums are POSIX `cksum`
//! CRCs. See `compatibility/apple-tools/mkbom.md`.
#![forbid(unsafe_code)]

mod cksum;
mod read;
mod write;

pub use cksum::{cksum, Cksum};
pub use read::read;
pub use write::write;

/// One CPU slice of a Mach-O file, as a BOM records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arch {
    pub cpu_type: u32,
    pub cpu_subtype: u32,
    pub size: u32,
    pub checksum: u32,
}

/// What a path is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File {
        size: u64,
        checksum: u32,
        /// CPU slices, for Mach-O files.
        archs: Vec<Arch>,
    },
    Symlink {
        target: String,
        checksum: u32,
    },
}

/// One BOM entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path relative to the root, without `./`; empty for the root itself.
    pub path: String,
    pub kind: Kind,
    /// Permission bits (the file type comes from `kind`).
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u32,
}

impl Entry {
    /// The full `st_mode`, with the file type.
    pub fn full_mode(&self) -> u16 {
        let kind = match self.kind {
            Kind::Directory => 0o040000,
            Kind::File { .. } => 0o100000,
            Kind::Symlink { .. } => 0o120000,
        };
        kind | (self.mode & 0o7777)
    }

    /// One `lsbom` line: path, octal mode, `uid/gid`, and for files and
    /// symlinks the size and checksum (and a symlink's target).
    pub fn lsbom_line(&self) -> String {
        let path = if self.path.is_empty() {
            ".".to_owned()
        } else {
            format!("./{}", self.path)
        };
        let mode = format!("{:o}", self.full_mode());
        match &self.kind {
            Kind::Directory => format!("{path}\t{mode}\t{}/{}", self.uid, self.gid),
            Kind::File { size, checksum, .. } => {
                format!(
                    "{path}\t{mode}\t{}/{}\t{size}\t{checksum}",
                    self.uid, self.gid
                )
            }
            Kind::Symlink { target, checksum } => format!(
                "{path}\t{mode}\t{}/{}\t{}\t{checksum}\t{target}",
                self.uid,
                self.gid,
                target.len()
            ),
        }
    }
}

/// Reads the CPU slices of a Mach-O image, thin or universal, with each
/// slice's size and `cksum`. Returns nothing for other files.
pub fn macho_archs(bytes: &[u8]) -> Vec<Arch> {
    let be = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let le = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let slice = |cpu_type: u32, cpu_subtype: u32, data: &[u8]| Arch {
        cpu_type,
        cpu_subtype,
        size: data.len() as u32,
        checksum: cksum(data),
    };
    match be(0) {
        Some(0xcafe_babe) => {
            let count = be(4).unwrap_or(0) as usize;
            (0..count.min(32))
                .filter_map(|i| {
                    let at = 8 + i * 20;
                    let (cpu, sub, offset, size) = (
                        be(at)?,
                        be(at + 4)?,
                        be(at + 8)? as usize,
                        be(at + 12)? as usize,
                    );
                    Some(slice(
                        cpu,
                        sub,
                        bytes.get(offset..offset.checked_add(size)?)?,
                    ))
                })
                .collect()
        }
        _ => match le(0) {
            Some(0xfeed_facf | 0xfeed_face) => match (le(4), le(8)) {
                (Some(cpu), Some(sub)) => vec![slice(cpu, sub, bytes)],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests;
