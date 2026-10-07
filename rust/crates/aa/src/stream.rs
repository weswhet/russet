//! The compressed stream `aa archive` writes: a `pbz` magic whose fourth
//! byte names the codec, the block size, then chunks of (decoded size,
//! stored size, data). A chunk whose stored size equals its decoded size is
//! stored uncompressed.

use std::io::{self, Read};

/// Largest decoded chunk accepted, well above the 4 MiB `aa` uses.
const MAX_CHUNK: u64 = 64 << 20;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Codec {
    Lzfse,
    Zlib,
    Xz,
    Lz4,
}

/// Returns the codec for a stream magic, or an error for a `pbz` stream
/// Russet can't decode.
fn codec(magic: &[u8; 4]) -> io::Result<Option<Codec>> {
    if &magic[..3] != b"pbz" {
        return Ok(None);
    }
    match magic[3] {
        b'e' => Ok(Some(Codec::Lzfse)),
        b'z' => Ok(Some(Codec::Zlib)),
        b'x' => Ok(Some(Codec::Xz)),
        b'4' => Ok(Some(Codec::Lz4)),
        b'b' => Err(invalid(
            "Apple Archive streams compressed with LZBITMAP aren't supported natively",
        )),
        other => Err(invalid(format!(
            "Unknown Apple Archive compression 'pbz{}'",
            other as char
        ))),
    }
}

/// True when `bytes` start a compressed Apple Archive stream (`pbze`,
/// `pbzz`, `pbz4`, or `pbzb`). `pbzx` is left to callers, because package
/// payloads in that format usually hold cpio archives.
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[..3] == b"pbz" && bytes[3] != b'x'
}

/// Decodes a `pbz` stream into the bytes it wraps.
pub struct Decoder<R> {
    inner: R,
    codec: Codec,
    chunk: Vec<u8>,
    position: usize,
    done: bool,
}

impl<R: Read> Decoder<R> {
    /// Reads the stream header. Fails unless the stream starts with a `pbz`
    /// magic Russet can decode.
    pub fn new(mut inner: R) -> io::Result<Self> {
        let mut header = [0u8; 12];
        inner.read_exact(&mut header)?;
        let codec = codec(header[..4].try_into().unwrap())?
            .ok_or_else(|| invalid("Not a compressed Apple Archive stream"))?;
        Ok(Self {
            inner,
            codec,
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
            return Err(invalid("Apple Archive chunk is too large"));
        }
        let mut stored = vec![0u8; length as usize];
        self.inner.read_exact(&mut stored)?;
        self.position = 0;
        self.chunk = if length == size {
            stored
        } else {
            decode(self.codec, &stored, size as usize)?
        };
        if self.chunk.len() as u64 != size {
            return Err(invalid("Apple Archive chunk size doesn't match its header"));
        }
        Ok(true)
    }
}

impl<R: Read> Read for Decoder<R> {
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

fn decode(codec: Codec, stored: &[u8], size: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    match codec {
        Codec::Lzfse => {
            lzfse_rust::decode_bytes(stored, &mut out)
                .map_err(|e| invalid(format!("Invalid LZFSE chunk: {e}")))?;
        }
        Codec::Zlib => {
            flate2::read::ZlibDecoder::new(stored)
                .take(size as u64 + 1)
                .read_to_end(&mut out)?;
        }
        Codec::Xz => {
            xz2::read::XzDecoder::new(stored)
                .take(size as u64 + 1)
                .read_to_end(&mut out)?;
        }
        Codec::Lz4 => decode_lz4(stored, size, &mut out)?,
    }
    Ok(out)
}

/// Apple's LZ4 framing: `bv41` blocks (decoded size, stored size, raw LZ4
/// block), `bv4-` uncompressed blocks (size, bytes), and a `bv4$` end mark.
fn decode_lz4(mut stored: &[u8], size: usize, out: &mut Vec<u8>) -> io::Result<()> {
    let word = |bytes: &[u8], at: usize| -> io::Result<usize> {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            .ok_or_else(|| invalid("Truncated LZ4 block"))
    };
    loop {
        let magic = stored
            .get(..4)
            .ok_or_else(|| invalid("Truncated LZ4 block"))?;
        match magic {
            b"bv4$" => return Ok(()),
            b"bv41" => {
                let (decoded, length) = (word(stored, 4)?, word(stored, 8)?);
                let block = stored
                    .get(12..12 + length)
                    .ok_or_else(|| invalid("Truncated LZ4 block"))?;
                if out.len() + decoded > size {
                    return Err(invalid("LZ4 block is larger than its chunk"));
                }
                let start = out.len();
                out.resize(start + decoded, 0);
                let written = lz4_flex::block::decompress_into(block, &mut out[start..])
                    .map_err(|e| invalid(format!("Invalid LZ4 block: {e}")))?;
                if written != decoded {
                    return Err(invalid("LZ4 block size doesn't match its header"));
                }
                stored = &stored[12 + length..];
            }
            b"bv4-" => {
                let length = word(stored, 4)?;
                let block = stored
                    .get(8..8 + length)
                    .ok_or_else(|| invalid("Truncated LZ4 block"))?;
                if out.len() + length > size {
                    return Err(invalid("LZ4 block is larger than its chunk"));
                }
                out.extend_from_slice(block);
                stored = &stored[8 + length..];
            }
            _ => return Err(invalid("Unknown LZ4 block type")),
        }
    }
}
