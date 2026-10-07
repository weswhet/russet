//! Shared helpers for the fuzz targets.
#![allow(dead_code)]

use russet_fs::Limits;
use std::io::Write;

/// Small limits, so one input can't fill the disk or take minutes.
pub fn limits() -> Limits {
    Limits {
        max_total_bytes: 16 << 20,
        max_entries: 10_000,
        ..Limits::default()
    }
}

/// Writes `data` to a temporary file for APIs that take a path.
pub fn file(data: &[u8]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(data).unwrap();
    file.flush().unwrap();
    file
}
