//! Native replacement for the `aa extract` Russet uses: reading Apple
//! Archive (`.aar`) streams, compressed with LZFSE, zlib, LZMA, or LZ4, or
//! stored uncompressed. LZBITMAP-compressed streams aren't supported.
//! See `compatibility/apple-tools/aa.md`.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod archive;
#[cfg(unix)]
mod stream;

#[cfg(unix)]
pub use imp::*;
#[cfg(unix)]
pub use stream::{is_compressed, Decoder};

#[cfg(unix)]
mod imp {
    use russet_fs::{Limits, SkippedXattr};
    use std::fs::File;
    use std::io::{self, BufReader, Read};
    use std::path::Path;

    /// True when `bytes` start an uncompressed Apple Archive entry.
    pub fn is_archive(bytes: &[u8]) -> bool {
        bytes.starts_with(b"AA01") || bytes.starts_with(b"YAA1")
    }

    /// True when `bytes` start an Apple Archive, compressed or not, other
    /// than a `pbzx` stream (which may hold a cpio archive instead).
    pub fn is_apple_archive(bytes: &[u8]) -> bool {
        is_archive(bytes) || crate::is_compressed(bytes)
    }

    fn open(source: &Path) -> io::Result<Box<dyn Read>> {
        let mut magic = [0u8; 4];
        let read = File::open(source)?.read(&mut magic)?;
        let file = BufReader::new(File::open(source)?);
        // An explicit Apple Archive may also be LZMA-compressed (`pbzx`).
        if crate::is_compressed(&magic[..read]) || magic[..read] == *b"pbzx" {
            Ok(Box::new(BufReader::new(crate::Decoder::new(file)?)))
        } else if is_archive(&magic[..read]) {
            Ok(Box::new(file))
        } else {
            Err(crate::stream::invalid("Not an Apple Archive"))
        }
    }

    /// Extracts an Apple Archive file like `aa extract -i source -d
    /// destination`. `destination` is created when missing.
    pub fn extract(
        source: &Path,
        destination: &Path,
        limits: Limits,
    ) -> io::Result<Vec<SkippedXattr>> {
        std::fs::create_dir_all(destination)?;
        crate::archive::extract(open(source)?, destination, limits)
    }

    /// Extracts an uncompressed Apple Archive stream, such as one a caller
    /// has already decompressed from a `pbzx` payload.
    pub fn extract_stream(
        reader: impl Read,
        destination: &Path,
        limits: Limits,
    ) -> io::Result<Vec<SkippedXattr>> {
        std::fs::create_dir_all(destination)?;
        crate::archive::extract(reader, destination, limits)
    }

    /// Reads one regular file from an Apple Archive file without extracting
    /// the rest. Returns `None` when the archive has no such file, and an
    /// error when it's larger than `max_bytes`.
    pub fn read_member(
        source: &Path,
        member: &Path,
        max_bytes: u64,
    ) -> io::Result<Option<Vec<u8>>> {
        let target = russet_fs::clean_relative(member)?;
        crate::archive::read_member(open(source)?, &target, max_bytes)
    }

    /// Like [`read_member`], for an uncompressed stream.
    pub fn read_member_stream(
        reader: impl Read,
        member: &Path,
        max_bytes: u64,
    ) -> io::Result<Option<Vec<u8>>> {
        let target = russet_fs::clean_relative(member)?;
        crate::archive::read_member(reader, &target, max_bytes)
    }
}

#[cfg(all(test, unix))]
mod tests;
