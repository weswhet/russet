//! The POSIX `cksum` CRC, which BOM files use for checksums.

const POLYNOMIAL: u32 = 0x04c1_1db7;

const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ POLYNOMIAL
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// An incremental POSIX `cksum`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cksum {
    crc: u32,
    length: u64,
}

impl Cksum {
    pub fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.crc = (self.crc << 8) ^ TABLE[((self.crc >> 24) as u8 ^ byte) as usize];
        }
        self.length += bytes.len() as u64;
    }

    /// The checksum, which also covers the length.
    pub fn finish(self) -> u32 {
        let mut crc = self.crc;
        let mut length = self.length;
        while length != 0 {
            crc = (crc << 8) ^ TABLE[((crc >> 24) as u8 ^ length as u8) as usize];
            length >>= 8;
        }
        !crc
    }
}

/// The POSIX `cksum` of `bytes`.
pub fn cksum(bytes: &[u8]) -> u32 {
    let mut sum = Cksum::default();
    sum.update(bytes);
    sum.finish()
}
