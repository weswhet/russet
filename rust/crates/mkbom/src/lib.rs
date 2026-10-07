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
use std::io::{self, Read, Write};
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

/// A file's size and checksums, as a BOM records them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub size: u64,
    pub checksum: u32,
    pub archs: Vec<Arch>,
}

/// The slices a Mach-O header describes: CPU type, subtype, offset, and
/// size (`None` for a thin file, whose slice is the whole file).
fn slices_in(header: &[u8]) -> Vec<(u32, u32, u64, Option<u64>)> {
    let be = |at: usize| {
        header
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let le = |at: usize| {
        header
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    match be(0) {
        Some(0xcafe_babe) => {
            let count = be(4).unwrap_or(0) as usize;
            (0..count.min(32))
                .filter_map(|i| {
                    let at = 8 + i * 20;
                    Some((
                        be(at)?,
                        be(at + 4)?,
                        u64::from(be(at + 8)?),
                        Some(u64::from(be(at + 12)?)),
                    ))
                })
                .collect()
        }
        _ => match le(0) {
            Some(0xfeed_facf | 0xfeed_face) => match (le(4), le(8)) {
                (Some(cpu), Some(sub)) => vec![(cpu, sub, 0, None)],
                _ => Vec::new(),
            },
            _ => Vec::new(),
        },
    }
}

/// Copies `reader` to `writer` and returns what a BOM records about it:
/// its size, `cksum`, and Mach-O slices, each with its size and `cksum`.
/// Reads the file once, without holding it in memory.
pub fn scan(mut reader: impl Read, mut writer: impl Write) -> io::Result<Summary> {
    // The universal header fits in the first chunk: 8 bytes and 20 per
    // slice, with at most 32 slices read.
    let mut buffer = vec![0u8; 1 << 16];
    let mut filled = 0;
    while filled < 8 + 32 * 20 {
        let read = reader.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    let slices = slices_in(&buffer[..filled]);
    let mut whole = Cksum::default();
    let mut sums = vec![Cksum::default(); slices.len()];
    let mut offset = 0u64;
    let mut chunk = filled;
    loop {
        let bytes = &buffer[..chunk];
        writer.write_all(bytes)?;
        whole.update(bytes);
        let end = offset + chunk as u64;
        for ((_, _, start, size), sum) in slices.iter().zip(&mut sums) {
            let stop = size.map_or(u64::MAX, |size| start.saturating_add(size));
            let (from, to) = ((*start).max(offset), stop.min(end));
            if from < to {
                sum.update(&bytes[(from - offset) as usize..(to - offset) as usize]);
            }
        }
        offset = end;
        chunk = reader.read(&mut buffer)?;
        if chunk == 0 {
            break;
        }
    }
    let archs = slices
        .iter()
        .zip(sums)
        .filter_map(|(&(cpu_type, cpu_subtype, start, size), sum)| {
            let size = size.unwrap_or(offset);
            // A slice that runs past the end of the file isn't listed.
            if start.checked_add(size)? > offset {
                return None;
            }
            Some(Arch {
                cpu_type,
                cpu_subtype,
                size: size as u32,
                checksum: sum.finish(),
            })
        })
        .collect();
    Ok(Summary {
        size: offset,
        checksum: whole.finish(),
        archs,
    })
}

/// Reads the CPU slices of a Mach-O image, thin or universal, with each
/// slice's size and `cksum`. Returns nothing for other files.
pub fn macho_archs(bytes: &[u8]) -> Vec<Arch> {
    scan(bytes, io::sink()).map(|s| s.archs).unwrap_or_default()
}

#[cfg(test)]
mod tests;
