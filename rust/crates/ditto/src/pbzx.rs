//! pbzx, the chunked container Apple uses for package payloads that
//! `pkgbuild --compression latest` writes: a `pbzx` magic and chunk size,
//! then chunks of xz-compressed (or raw) data that concatenate to a cpio
//! archive.

use std::io::{self, Read};

const MAGIC: &[u8; 4] = b"pbzx";
const XZ_MAGIC: &[u8; 6] = b"\xfd7zXZ\0";
/// Largest decompressed chunk accepted, well above the 16 MiB Apple uses.
const MAX_CHUNK: u64 = 64 << 20;

fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Invalid pbzx payload: {message}"),
    )
}

/// Returns true when `bytes` start with the pbzx magic.
pub(crate) fn is_pbzx(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Decodes a pbzx stream into the bytes it wraps.
pub(crate) struct PbzxReader<R> {
    inner: R,
    chunk: Vec<u8>,
    position: usize,
    done: bool,
}

impl<R: Read> PbzxReader<R> {
    pub(crate) fn new(mut inner: R) -> io::Result<Self> {
        let mut header = [0u8; 12];
        inner.read_exact(&mut header)?;
        if &header[..4] != MAGIC {
            return Err(invalid("missing magic"));
        }
        Ok(Self {
            inner,
            chunk: Vec::new(),
            position: 0,
            done: false,
        })
    }

    fn next_chunk(&mut self) -> io::Result<bool> {
        let mut header = [0u8; 16];
        match self.inner.read(&mut header[..1])? {
            0 => return Ok(false),
            _ => self.inner.read_exact(&mut header[1..])?,
        }
        let size = u64::from_be_bytes(header[..8].try_into().unwrap());
        let length = u64::from_be_bytes(header[8..].try_into().unwrap());
        if size > MAX_CHUNK || length > MAX_CHUNK {
            return Err(invalid("chunk too large"));
        }
        let mut stored = vec![0u8; length as usize];
        self.inner.read_exact(&mut stored)?;
        self.chunk.clear();
        self.position = 0;
        if stored.starts_with(XZ_MAGIC) {
            xz2::read::XzDecoder::new(stored.as_slice())
                .take(size + 1)
                .read_to_end(&mut self.chunk)?;
            if self.chunk.len() as u64 != size {
                return Err(invalid("chunk size doesn't match its header"));
            }
        } else {
            self.chunk = stored;
        }
        Ok(true)
    }
}

impl<R: Read> Read for PbzxReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position == self.chunk.len() {
            if self.done || !self.next_chunk()? {
                self.done = true;
                return Ok(0);
            }
        }
        let count = buf.len().min(self.chunk.len() - self.position);
        buf[..count].copy_from_slice(&self.chunk[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
