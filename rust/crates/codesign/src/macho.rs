//! Finds the code signature in a Mach-O file, thin or universal, and parses
//! its blobs: code directories, requirements, entitlements, and the CMS
//! signature.

const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const MH_MAGIC: u32 = 0xfeed_face;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_SEGMENT: u32 = 0x1;
const LC_SEGMENT_64: u32 = 0x19;

/// Reads a 32-bit Mach-O header field in the slice's byte order.
fn field(image: &[u8], at: usize, big: bool) -> Result<u32, String> {
    image
        .get(at..at + 4)
        .map(|b| {
            let b = b.try_into().unwrap();
            if big {
                u32::from_be_bytes(b)
            } else {
                u32::from_le_bytes(b)
            }
        })
        .ok_or_else(|| error("truncated load commands"))
}

/// Finds `__TEXT,__info_plist` in the segment command at `at`.
fn info_section(image: &[u8], at: usize, wide: bool, big: bool) -> Result<Option<&[u8]>, String> {
    let name = |offset: usize| -> &[u8] {
        let bytes = image.get(offset..offset + 16).unwrap_or_default();
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        &bytes[..end]
    };
    let read = |offset: usize| field(image, offset, big);
    if name(at + 8) != b"__TEXT" {
        return Ok(None);
    }
    let (sections, first, stride) = if wide {
        (read(at + 64)? as usize, at + 72, 80)
    } else {
        (read(at + 48)? as usize, at + 56, 68)
    };
    for i in 0..sections.min(256) {
        let section = first + i * stride;
        if name(section) == b"__info_plist" {
            let (size, offset) = if wide {
                (read(section + 40)? as usize, read(section + 48)? as usize)
            } else {
                (read(section + 36)? as usize, read(section + 40)? as usize)
            };
            return Ok(image.get(offset..offset + size));
        }
    }
    Ok(None)
}

pub(crate) const SUPERBLOB: u32 = 0xfade_0cc0;
pub(crate) const CODE_DIRECTORY: u32 = 0xfade_0c02;

/// Slot numbers in a superblob's index.
pub(crate) const SLOT_CODE_DIRECTORY: u32 = 0;
pub(crate) const SLOT_REQUIREMENTS: u32 = 2;
pub(crate) const SLOT_ENTITLEMENTS: u32 = 5;
pub(crate) const SLOT_DER_ENTITLEMENTS: u32 = 7;
pub(crate) const SLOT_ALTERNATE_DIRECTORIES: u32 = 0x1000;
pub(crate) const SLOT_SIGNATURE: u32 = 0x10000;

fn error(message: &str) -> String {
    format!("Invalid code signature: {message}")
}

fn be32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| error("truncated"))
}

fn be64(bytes: &[u8], at: usize) -> Result<u64, String> {
    bytes
        .get(at..at + 8)
        .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| error("truncated"))
}

/// One architecture of a Mach-O file.
pub struct Slice<'a> {
    /// The `__TEXT,__info_plist` section, which a standalone executable's
    /// signature seals in place of a bundle's Info.plist.
    pub info_plist: Option<&'a [u8]>,
    /// The whole thin Mach-O image.
    pub image: &'a [u8],
    /// Offset of the signature superblob within `image`; code pages cover
    /// everything before it.
    pub signature_offset: usize,
    pub signature: &'a [u8],
}

/// Splits a Mach-O file into its architectures. Fails when the file isn't
/// Mach-O or an architecture is unsigned.
pub fn slices(file: &[u8]) -> Result<Vec<Slice<'_>>, String> {
    let magic = be32(file, 0).map_err(|_| error("not a Mach-O file"))?;
    if magic == FAT_MAGIC || magic == FAT_MAGIC_64 {
        let count = be32(file, 4)? as usize;
        if count == 0 || count > 32 {
            return Err(error("bad universal header"));
        }
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let (offset, size) = if magic == FAT_MAGIC {
                let at = 8 + i * 20;
                (be32(file, at + 8)? as usize, be32(file, at + 12)? as usize)
            } else {
                let at = 8 + i * 32;
                (be64(file, at + 8)? as usize, be64(file, at + 16)? as usize)
            };
            let image = file
                .get(offset..offset.checked_add(size).ok_or_else(|| error("bad slice"))?)
                .ok_or_else(|| error("slice out of range"))?;
            out.push(thin(image)?);
        }
        return Ok(out);
    }
    Ok(vec![thin(file)?])
}

fn thin(image: &[u8]) -> Result<Slice<'_>, String> {
    let magic = be32(image, 0).map_err(|_| error("not a Mach-O file"))?;
    // PowerPC slices in older universal binaries are big-endian. codesign
    // signs and verifies them like any other slice; the signature blobs
    // themselves are big-endian either way.
    let (header, big) = match magic {
        m if m == MH_MAGIC_64.swap_bytes() => (32, false),
        m if m == MH_MAGIC.swap_bytes() => (28, false),
        MH_MAGIC_64 => (32, true),
        MH_MAGIC => (28, true),
        _ => return Err(error("not a Mach-O file")),
    };
    let read = |at: usize| field(image, at, big);
    let count = read(16)? as usize;
    let mut at = header;
    let mut found = None;
    let mut info_plist = None;
    for _ in 0..count {
        let command = read(at)?;
        let size = read(at + 4)? as usize;
        if size < 8 {
            return Err(error("bad load command"));
        }
        if command == LC_CODE_SIGNATURE {
            let offset = read(at + 8)? as usize;
            let length = read(at + 12)? as usize;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| error("bad signature range"))?;
            let signature = image
                .get(offset..end)
                .ok_or_else(|| error("signature out of range"))?;
            found = Some((offset, signature));
        } else if (command == LC_SEGMENT_64 || command == LC_SEGMENT) && info_plist.is_none() {
            info_plist = info_section(image, at, command == LC_SEGMENT_64, big)?;
        }
        at = at
            .checked_add(size)
            .ok_or_else(|| error("bad load command"))?;
    }
    let (signature_offset, signature) = found.ok_or("code object is not signed at all")?;
    Ok(Slice {
        info_plist,
        image,
        signature_offset,
        signature,
    })
}

/// The blobs in a superblob, by slot.
pub fn blobs(superblob: &[u8]) -> Result<Vec<(u32, &[u8])>, String> {
    if be32(superblob, 0)? != SUPERBLOB {
        return Err(error("missing superblob"));
    }
    let length = be32(superblob, 4)? as usize;
    let superblob = superblob
        .get(..length)
        .ok_or_else(|| error("superblob out of range"))?;
    let count = be32(superblob, 8)? as usize;
    if count > 64 {
        return Err(error("too many blobs"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let slot = be32(superblob, 12 + i * 8)?;
        let offset = be32(superblob, 16 + i * 8)? as usize;
        let size = be32(superblob, offset + 4)? as usize;
        let blob = superblob
            .get(offset..offset.checked_add(size).ok_or_else(|| error("bad blob"))?)
            .ok_or_else(|| error("blob out of range"))?;
        out.push((slot, blob));
    }
    Ok(out)
}

/// A parsed code directory.
#[derive(Clone, Debug)]
pub struct CodeDirectory<'a> {
    pub bytes: &'a [u8],
    pub flags: u32,
    pub identifier: String,
    pub team: Option<String>,
    pub hash_type: u8,
    pub hash_size: usize,
    pub page_size: usize,
    pub code_limit: u64,
    hash_offset: usize,
    special_slots: u32,
    code_slots: u32,
}

/// Code directory hash types.
pub const HASH_SHA1: u8 = 1;
pub const HASH_SHA256: u8 = 2;
pub const HASH_SHA256_TRUNCATED: u8 = 3;
pub const HASH_SHA384: u8 = 4;

pub fn hash(kind: u8, bytes: &[u8]) -> Result<Vec<u8>, String> {
    use sha2::Digest;
    Ok(match kind {
        HASH_SHA1 => sha1::Sha1::digest(bytes).to_vec(),
        HASH_SHA256 => sha2::Sha256::digest(bytes).to_vec(),
        HASH_SHA256_TRUNCATED => sha2::Sha256::digest(bytes)[..20].to_vec(),
        HASH_SHA384 => sha2::Sha384::digest(bytes).to_vec(),
        other => return Err(format!("unsupported code directory hash type {other}")),
    })
}

fn c_string(bytes: &[u8], at: usize) -> Result<String, String> {
    let rest = bytes
        .get(at..)
        .ok_or_else(|| error("string out of range"))?;
    let end = rest
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| error("unterminated string"))?;
    String::from_utf8(rest[..end].to_vec()).map_err(|_| error("string isn't UTF-8"))
}

impl<'a> CodeDirectory<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if be32(bytes, 0)? != CODE_DIRECTORY {
            return Err(error("not a code directory"));
        }
        let version = be32(bytes, 8)?;
        let flags = be32(bytes, 12)?;
        let hash_offset = be32(bytes, 16)? as usize;
        let ident_offset = be32(bytes, 20)? as usize;
        let special_slots = be32(bytes, 24)?;
        let code_slots = be32(bytes, 28)?;
        let mut code_limit = u64::from(be32(bytes, 32)?);
        let hash_size = *bytes.get(36).ok_or_else(|| error("truncated"))? as usize;
        let hash_type = *bytes.get(37).ok_or_else(|| error("truncated"))?;
        let page_shift = *bytes.get(39).ok_or_else(|| error("truncated"))?;
        let team = if version >= 0x20200 {
            match be32(bytes, 48)? as usize {
                0 => None,
                at => Some(c_string(bytes, at)?),
            }
        } else {
            None
        };
        if version >= 0x20300 {
            let limit64 = be64(bytes, 56)?;
            if limit64 != 0 {
                code_limit = limit64;
            }
        }
        if hash_size != hash(hash_type, b"")?.len() || page_shift > 30 || special_slots > 64 {
            return Err(error("bad code directory geometry"));
        }
        let first = hash_offset
            .checked_sub(special_slots as usize * hash_size)
            .ok_or_else(|| error("bad hash offset"))?;
        let end = hash_offset + code_slots as usize * hash_size;
        if first < 44 || end > bytes.len() {
            return Err(error("hash slots out of range"));
        }
        Ok(Self {
            bytes,
            flags,
            identifier: c_string(bytes, ident_offset)?,
            team,
            hash_type,
            hash_size,
            page_size: if page_shift == 0 { 0 } else { 1 << page_shift },
            code_limit,
            hash_offset,
            special_slots,
            code_slots,
        })
    }

    /// The code directory hash: its digest, truncated to 20 bytes.
    pub fn cdhash(&self) -> Result<Vec<u8>, String> {
        Ok(hash(self.hash_type, self.bytes)?[..20].to_vec())
    }

    /// The hash recorded in special slot `slot` (1 for Info.plist, 2 for
    /// requirements, 3 for resources, 5 for entitlements), or `None` when
    /// the slot is absent or zero.
    pub fn special(&self, slot: u32) -> Option<&'a [u8]> {
        if slot == 0 || slot > self.special_slots {
            return None;
        }
        let at = self.hash_offset - slot as usize * self.hash_size;
        let value = &self.bytes[at..at + self.hash_size];
        value.iter().any(|b| *b != 0).then_some(value)
    }

    /// Checks every code page hash against `image` up to the code limit.
    pub fn verify_pages(&self, image: &[u8]) -> Result<(), String> {
        let limit = usize::try_from(self.code_limit).map_err(|_| error("code limit too large"))?;
        let code = image
            .get(..limit)
            .ok_or_else(|| error("code limit beyond the file"))?;
        // A file with no page size is one page, unless it is empty, which
        // codesign signs with no pages.
        let pages = if self.page_size == 0 {
            usize::from(!code.is_empty())
        } else {
            code.len().div_ceil(self.page_size)
        };
        if pages == 0 && !image.is_empty() {
            return Err(error("code limit covers none of the file"));
        }
        if pages != self.code_slots as usize {
            return Err(error("page count doesn't match the code directory"));
        }
        for page in 0..pages {
            let chunk = if self.page_size == 0 {
                code
            } else {
                &code[page * self.page_size..((page + 1) * self.page_size).min(code.len())]
            };
            let at = self.hash_offset + page * self.hash_size;
            if hash(self.hash_type, chunk)? != self.bytes[at..at + self.hash_size] {
                return Err("code page hash doesn't match; the executable was modified".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32-bit thin image with only an LC_CODE_SIGNATURE command, in either
    /// byte order. The superblob stays big-endian, as Mach-O requires.
    fn image(big: bool) -> Vec<u8> {
        let word = |v: u32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut out = Vec::new();
        // Header: magic, cputype (ppc or i386), subtype, filetype, ncmds,
        // sizeofcmds, flags.
        for v in [MH_MAGIC, if big { 18 } else { 7 }, 0, 2, 1, 16, 0] {
            out.extend(word(v));
        }
        // LC_CODE_SIGNATURE: the signature starts right after it.
        for v in [LC_CODE_SIGNATURE, 16, 44, 12] {
            out.extend(word(v));
        }
        out.extend(SUPERBLOB.to_be_bytes());
        out.extend(12u32.to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out
    }

    #[test]
    fn reads_big_endian_powerpc_slices() {
        for big in [false, true] {
            let file = image(big);
            let slices = slices(&file).unwrap();
            assert_eq!(slices.len(), 1);
            assert_eq!(slices[0].signature_offset, 44, "big={big}");
            assert_eq!(slices[0].signature, &file[44..], "big={big}");
        }
        assert!(slices(b"\0\0\0\0").is_err());
    }
}
