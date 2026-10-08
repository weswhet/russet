//! Native replacement for the `ditto` extraction modes Russet uses.
//!
//! - [`extract_zip`] matches `ditto -x -k --noqtn`: it keeps Unix modes and
//!   symlinks, and merges AppleDouble members (in `__MACOSX/` or stored next
//!   to the file as `._name`) back into extended attributes.
//! - [`extract_cpio`] matches `ditto -x --noqtn` for cpio archives, plain or
//!   gzip-compressed, as found in package payloads and `.cpgz` files. It
//!   also reads pbzx payloads, which `ditto` can't but `aa extract` can.
//!
//! Both write through [`russet_fs::TreeWriter`], so archive content can't
//! escape the destination.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod appledouble;
#[cfg(unix)]
mod cpio;
#[cfg(unix)]
mod cpio_write;
#[cfg(unix)]
mod pbzx;
#[cfg(all(test, unix))]
mod tests;
#[cfg(unix)]
mod zip;

#[cfg(unix)]
pub use appledouble::encode as encode_apple_double;
pub use cpio_write::{write_tree, CpioWriter, Header};
#[cfg(unix)]
pub use imp::*;

#[cfg(unix)]
mod imp {
    use russet_fs::{Limits, SkippedXattr};
    use std::fs::{self, File};
    use std::io::{self, BufRead, BufReader, Read};
    use std::path::Path;

    /// The result of an extraction.
    #[derive(Debug, Default)]
    pub struct Report {
        /// Extended attributes the host filesystem refused, such as resource
        /// forks on symlinks under Linux.
        pub skipped_xattrs: Vec<SkippedXattr>,
    }

    /// Extracts a zip archive like `ditto -x -k --noqtn source destination`.
    /// `destination` is created when missing.
    pub fn extract_zip(source: &Path, destination: &Path, limits: Limits) -> io::Result<Report> {
        fs::create_dir_all(destination)?;
        Ok(Report {
            skipped_xattrs: crate::zip::extract(source, destination, limits)?,
        })
    }

    /// Extracts a cpio archive, plain or gzip-compressed, like
    /// `ditto -x --noqtn source destination`. `destination` is created when
    /// missing.
    pub fn extract_cpio(source: &Path, destination: &Path, limits: Limits) -> io::Result<Report> {
        extract_cpio_with(source, destination, limits, true)
    }

    /// Reads one regular file from a cpio archive (plain, gzip, or pbzx)
    /// without extracting the rest, like `tar -xOf source member`. Returns
    /// `None` when the archive has no such file, and an error when it's
    /// larger than `max_bytes`.
    pub fn read_cpio_member(
        source: &Path,
        member: &Path,
        max_bytes: u64,
    ) -> io::Result<Option<Vec<u8>>> {
        let target = russet_fs::clean_relative(member)?;
        let mut file = File::open(source)?;
        let mut magic = [0; 4];
        let read = file.read(&mut magic)?;
        let file = BufReader::new(File::open(source)?);
        if russet_aa::is_apple_archive(&magic[..read]) {
            russet_aa::read_member(source, member, max_bytes)
        } else if read >= 2 && magic[..2] == [0x1f, 0x8b] {
            crate::cpio::read_member(
                flate2::bufread::MultiGzDecoder::new(file),
                &target,
                max_bytes,
            )
        } else if crate::pbzx::is_pbzx(&magic[..read]) {
            let mut payload = BufReader::new(crate::pbzx::PbzxReader::new(file)?);
            if russet_aa::is_archive(payload.fill_buf()?) {
                russet_aa::read_member_stream(payload, member, max_bytes)
            } else {
                crate::cpio::read_member(payload, &target, max_bytes)
            }
        } else {
            crate::cpio::read_member(file, &target, max_bytes)
        }
    }

    /// Like [`extract_cpio`], but with `apple_double` false, `._name` members
    /// stay ordinary files, as `pkgutil --expand` leaves them in Scripts.
    pub fn extract_cpio_with(
        source: &Path,
        destination: &Path,
        limits: Limits,
        apple_double: bool,
    ) -> io::Result<Report> {
        fs::create_dir_all(destination)?;
        let mut file = File::open(source)?;
        let mut magic = [0; 4];
        let read = file.read(&mut magic)?;
        let file = BufReader::new(File::open(source)?);
        let skipped = if russet_aa::is_apple_archive(&magic[..read]) {
            // `ditto` can't read Apple Archive payloads; `aa extract` can.
            russet_aa::extract(source, destination, limits)?
        } else if read >= 2 && magic[..2] == [0x1f, 0x8b] {
            crate::cpio::extract(
                flate2::bufread::MultiGzDecoder::new(file),
                destination,
                limits,
                apple_double,
            )?
        } else if crate::pbzx::is_pbzx(&magic[..read]) {
            // macOS reads pbzx payloads with `aa extract`, which keeps
            // `._name` members as ordinary files. The payload inside is cpio
            // or an Apple Archive.
            let mut payload = BufReader::new(crate::pbzx::PbzxReader::new(file)?);
            if russet_aa::is_archive(payload.fill_buf()?) {
                russet_aa::extract_stream(payload, destination, limits)?
            } else {
                crate::cpio::extract(payload, destination, limits, false)?
            }
        } else {
            crate::cpio::extract(file, destination, limits, apple_double)?
        };
        Ok(Report {
            skipped_xattrs: skipped,
        })
    }
}
