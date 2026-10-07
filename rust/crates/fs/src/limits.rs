/// Bounds on what one extraction may write.
///
/// The defaults allow the largest vendor installers seen in practice while
/// stopping decompression bombs and pathological trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Total bytes written to regular files and extended attributes.
    pub max_total_bytes: u64,
    /// Number of entries of any kind.
    pub max_entries: u64,
    /// Number of path components below the destination.
    pub max_depth: usize,
    /// Bytes in one path component.
    pub max_name_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_total_bytes: 64 << 30,
            max_entries: 4_000_000,
            max_depth: 256,
            max_name_bytes: 255,
        }
    }
}
