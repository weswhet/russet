use std::io;

/// A checksum algorithm xar records for the TOC or file data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Md5,
    Sha256,
    Sha512,
}

impl Algorithm {
    /// Parses a `style` attribute such as `sha1`.
    pub fn from_style(style: &str) -> Option<Self> {
        match style.to_ascii_lowercase().as_str() {
            "sha1" => Some(Self::Sha1),
            "md5" => Some(Self::Md5),
            "sha256" => Some(Self::Sha256),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    pub fn digest_len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Md5 => 16,
            Self::Sha256 => 32,
            Self::Sha512 => 64,
        }
    }

    pub(crate) fn hasher(self) -> Hasher {
        use sha2::Digest;
        match self {
            Self::Sha1 => Hasher::Sha1(sha1::Sha1::new()),
            Self::Md5 => Hasher::Md5(md5::Md5::new()),
            Self::Sha256 => Hasher::Sha256(sha2::Sha256::new()),
            Self::Sha512 => Hasher::Sha512(sha2::Sha512::new()),
        }
    }

    pub fn digest(self, bytes: &[u8]) -> Vec<u8> {
        let mut hasher = self.hasher();
        hasher.update(bytes);
        hasher.finish()
    }
}

pub(crate) enum Hasher {
    Sha1(sha1::Sha1),
    Md5(md5::Md5),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        use sha2::Digest;
        match self {
            Self::Sha1(h) => h.update(bytes),
            Self::Md5(h) => h.update(bytes),
            Self::Sha256(h) => h.update(bytes),
            Self::Sha512(h) => h.update(bytes),
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        use sha2::Digest;
        match self {
            Self::Sha1(h) => h.finalize().to_vec(),
            Self::Md5(h) => h.finalize().to_vec(),
            Self::Sha256(h) => h.finalize().to_vec(),
            Self::Sha512(h) => h.finalize().to_vec(),
        }
    }
}

/// A writer that hashes what passes through it.
pub(crate) struct HashingWriter<W> {
    pub(crate) inner: W,
    pub(crate) hasher: Option<Hasher>,
}

impl<W: io::Write> io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buf)?;
        if let Some(hasher) = &mut self.hasher {
            hasher.update(&buf[..count]);
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(crate) fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}
