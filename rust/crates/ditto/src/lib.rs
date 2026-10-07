//! Native replacement for the `ditto` extraction modes Russet uses.
//!
//! - [`extract_zip`] matches `ditto -x -k --noqtn`: it keeps Unix modes and
//!   symlinks, and merges AppleDouble members (in `__MACOSX/` or stored next
//!   to the file as `._name`) back into extended attributes.
//! - [`extract_cpio`] matches `ditto -x --noqtn` for cpio archives, plain or
//!   gzip-compressed, as found in package payloads and `.cpgz` files.
//!
//! Both write through [`russet_fs::TreeWriter`], so archive content can't
//! escape the destination.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod appledouble;
#[cfg(unix)]
mod cpio;
#[cfg(all(test, unix))]
mod tests;
#[cfg(unix)]
mod zip;

#[cfg(unix)]
pub use imp::*;

#[cfg(unix)]
mod imp {
    use russet_fs::{Limits, SkippedXattr};
    use std::fs::{self, File};
    use std::io::{self, BufReader, Read};
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
        fs::create_dir_all(destination)?;
        let mut file = File::open(source)?;
        let mut magic = [0; 2];
        let gzip = file.read(&mut magic)? == 2 && magic == [0x1f, 0x8b];
        let file = BufReader::new(File::open(source)?);
        let skipped = if gzip {
            crate::cpio::extract(
                flate2::bufread::MultiGzDecoder::new(file),
                destination,
                limits,
            )?
        } else {
            crate::cpio::extract(file, destination, limits)?
        };
        Ok(Report {
            skipped_xattrs: skipped,
        })
    }
}
