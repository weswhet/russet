use crate::digest::{decode_hex, Algorithm, HashingWriter};
use russet_fs::{clean_relative, Limits, SkippedXattr, TreeWriter};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"xar!";
const MAX_TOC_COMPRESSED: u64 = 64 << 20;
const MAX_TOC: u64 = 256 << 20;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// How an entry's data is compressed in the heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    None,
    /// `application/x-gzip`, which xar stores as a zlib stream.
    Zlib,
    Bzip2,
    Xz,
}

/// Where an entry's data lives in the heap, and how to check it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Data {
    /// Offset from the start of the heap.
    pub offset: u64,
    /// Bytes stored in the heap.
    pub length: u64,
    /// Bytes after decoding.
    pub size: u64,
    pub encoding: Encoding,
    pub archived_checksum: Option<(Algorithm, Vec<u8>)>,
    pub extracted_checksum: Option<(Algorithm, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    /// A hard link to the entry whose ID is in [`Entry::link`], or the
    /// original that other hard links name.
    HardLink,
    /// Devices, FIFOs, and other types Russet doesn't extract.
    Other,
}

/// One file in the archive's TOC.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// Path relative to the archive root.
    pub path: PathBuf,
    pub kind: EntryKind,
    pub mode: Option<u32>,
    pub data: Option<Data>,
    /// A symlink's target, or for a hard link the ID of the original
    /// (`original` for the original itself).
    pub link: Option<String>,
}

/// A signature over the TOC checksum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// `RSA` for `<signature>`, or `CMS` for `<x-signature>`.
    pub style: String,
    /// The signature bytes from the heap.
    pub bytes: Vec<u8>,
    /// DER certificates from `KeyInfo`, leaf first.
    pub certificates: Vec<Vec<u8>>,
}

/// An opened, validated xar archive.
pub struct Archive {
    file: File,
    heap_start: u64,
    toc_xml: String,
    checksum: Option<(Algorithm, Vec<u8>)>,
    entries: Vec<Entry>,
    signatures: Vec<Signature>,
}

fn text<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|c| c.has_tag_name(name))
        .and_then(|c| c.text())
        .map(str::trim)
}

fn number(node: roxmltree::Node, name: &str) -> io::Result<u64> {
    text(node, name)
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| invalid(format!("xar TOC element is missing a valid <{name}>")))
}

fn checksum(node: roxmltree::Node, name: &str) -> io::Result<Option<(Algorithm, Vec<u8>)>> {
    let Some(element) = node.children().find(|c| c.has_tag_name(name)) else {
        return Ok(None);
    };
    let algorithm = element
        .attribute("style")
        .and_then(Algorithm::from_style)
        .ok_or_else(|| invalid(format!("xar <{name}> has an unsupported style")))?;
    let value = element
        .text()
        .and_then(decode_hex)
        .filter(|v| v.len() == algorithm.digest_len())
        .ok_or_else(|| invalid(format!("xar <{name}> isn't a valid digest")))?;
    Ok(Some((algorithm, value)))
}

fn data(node: roxmltree::Node) -> io::Result<Option<Data>> {
    let Some(data) = node.children().find(|c| c.has_tag_name("data")) else {
        return Ok(None);
    };
    let encoding = match data
        .children()
        .find(|c| c.has_tag_name("encoding"))
        .and_then(|c| c.attribute("style"))
    {
        None | Some("application/octet-stream") => Encoding::None,
        Some("application/x-gzip") => Encoding::Zlib,
        Some("application/x-bzip2") => Encoding::Bzip2,
        Some("application/x-lzma" | "application/x-xz") => Encoding::Xz,
        Some(other) => return Err(invalid(format!("xar encoding {other} isn't supported"))),
    };
    Ok(Some(Data {
        offset: number(data, "offset")?,
        length: number(data, "length")?,
        size: number(data, "size")?,
        encoding,
        archived_checksum: checksum(data, "archived-checksum")?,
        extracted_checksum: checksum(data, "extracted-checksum")?,
    }))
}

fn files(
    parent: roxmltree::Node,
    prefix: &Path,
    entries: &mut Vec<Entry>,
    ids: &mut HashSet<String>,
) -> io::Result<()> {
    // Apple's xar lists siblings in ID order, which needn't be document
    // order: productbuild writes later components first.
    let mut nodes: Vec<_> = parent
        .children()
        .filter(|c| c.has_tag_name("file"))
        .collect();
    nodes.sort_by_key(|n| {
        let id = n.attribute("id").unwrap_or_default();
        (id.parse::<u64>().unwrap_or(u64::MAX), id.to_owned())
    });
    for node in nodes {
        let id = node
            .attribute("id")
            .ok_or_else(|| invalid("xar <file> has no id"))?
            .to_owned();
        if !ids.insert(id.clone()) {
            return Err(invalid(format!("xar TOC repeats file id {id}")));
        }
        let name = text(node, "name").ok_or_else(|| invalid("xar <file> has no name"))?;
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\0')
        {
            return Err(invalid(format!("xar entry has an invalid name {name:?}")));
        }
        let path = clean_relative(&prefix.join(name))?;
        let type_node = node.children().find(|c| c.has_tag_name("type"));
        let kind = match type_node.and_then(|t| t.text()).map(str::trim) {
            Some("file") | None => EntryKind::File,
            Some("directory") => EntryKind::Directory,
            Some("symlink") => EntryKind::Symlink,
            Some("hardlink") => EntryKind::HardLink,
            Some(_) => EntryKind::Other,
        };
        let link = match kind {
            EntryKind::Symlink => Some(
                text(node, "link")
                    .ok_or_else(|| invalid("xar symlink has no target"))?
                    .to_owned(),
            ),
            EntryKind::HardLink => Some(
                type_node
                    .and_then(|t| t.attribute("link"))
                    .ok_or_else(|| invalid("xar hard link has no link attribute"))?
                    .to_owned(),
            ),
            _ => None,
        };
        let mode = match text(node, "mode") {
            Some(mode) => Some(
                u32::from_str_radix(mode, 8)
                    .map_err(|_| invalid(format!("xar mode {mode:?} isn't octal")))?
                    & 0o7777,
            ),
            None => None,
        };
        entries.push(Entry {
            id,
            path: path.clone(),
            kind,
            mode,
            data: data(node)?,
            link,
        });
        if kind == EntryKind::Directory {
            files(node, &path, entries, ids)?;
        }
    }
    Ok(())
}

fn overlaps(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < b.0 + b.1 && b.0 < a.0 + a.1 && a.1 > 0 && b.1 > 0
}

impl Archive {
    /// Opens and validates an archive. Fails unless the TOC checksum matches
    /// and every heap reference is in range.
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut header = [0u8; 28];
        file.read_exact(&mut header)
            .map_err(|_| invalid(format!("{} isn't a xar archive", path.display())))?;
        if &header[..4] != MAGIC {
            return Err(invalid(format!("{} isn't a xar archive", path.display())));
        }
        let header_size = u64::from(u16::from_be_bytes([header[4], header[5]]));
        let toc_compressed = u64::from_be_bytes(header[8..16].try_into().unwrap());
        let toc_size = u64::from_be_bytes(header[16..24].try_into().unwrap());
        let algorithm_id = u32::from_be_bytes(header[24..28].try_into().unwrap());
        if header_size < 28 || toc_compressed > MAX_TOC_COMPRESSED || toc_size > MAX_TOC {
            return Err(invalid("xar header is invalid or its TOC is too large"));
        }
        let header_algorithm = match algorithm_id {
            0 => None,
            1 => Some(Algorithm::Sha1),
            2 => Some(Algorithm::Md5),
            3 => {
                let mut name = vec![0u8; (header_size - 28) as usize];
                file.read_exact(&mut name)?;
                let name = String::from_utf8_lossy(&name);
                Some(
                    Algorithm::from_style(name.trim_end_matches('\0'))
                        .ok_or_else(|| invalid(format!("xar checksum {name} isn't supported")))?,
                )
            }
            other => {
                return Err(invalid(format!(
                    "xar checksum type {other} isn't supported"
                )))
            }
        };
        file.seek(SeekFrom::Start(header_size))?;
        let mut compressed = vec![0u8; toc_compressed as usize];
        file.read_exact(&mut compressed)
            .map_err(|_| invalid("xar TOC is truncated"))?;
        let mut xml = Vec::with_capacity(toc_size as usize);
        flate2::read::ZlibDecoder::new(compressed.as_slice())
            .take(toc_size + 1)
            .read_to_end(&mut xml)
            .map_err(|_| invalid("xar TOC isn't valid zlib data"))?;
        if xml.len() as u64 != toc_size {
            return Err(invalid("xar TOC size doesn't match its header"));
        }
        let toc_xml = String::from_utf8(xml).map_err(|_| invalid("xar TOC isn't UTF-8"))?;
        let heap_start = header_size + toc_compressed;

        let document = roxmltree::Document::parse(&toc_xml)
            .map_err(|e| invalid(format!("xar TOC isn't valid XML: {e}")))?;
        let toc = document
            .root_element()
            .children()
            .find(|c| c.has_tag_name("toc"))
            .ok_or_else(|| invalid("xar TOC has no <toc> element"))?;
        if document
            .descendants()
            .filter(|n| n.has_tag_name("checksum"))
            .count()
            != toc
                .children()
                .filter(|n| n.has_tag_name("checksum"))
                .count()
        {
            return Err(invalid("xar TOC has a checksum element outside <toc>"));
        }
        let checksums: Vec<_> = toc
            .children()
            .filter(|n| n.has_tag_name("checksum"))
            .collect();
        let mut reserved = Vec::new();
        let checksum = match (header_algorithm, checksums.as_slice()) {
            (None, []) => None,
            (Some(algorithm), [element]) => {
                if element.attribute("style").and_then(Algorithm::from_style) != Some(algorithm) {
                    return Err(invalid("xar TOC checksum style doesn't match its header"));
                }
                let offset = number(*element, "offset")?;
                let size = number(*element, "size")?;
                if size != algorithm.digest_len() as u64 {
                    return Err(invalid("xar TOC checksum has the wrong size"));
                }
                let stored = read_range(&mut file, length, heap_start, offset, size)?;
                if stored != algorithm.digest(&compressed) {
                    return Err(invalid("xar TOC checksum doesn't match"));
                }
                reserved.push((offset, size));
                Some((algorithm, stored))
            }
            _ => return Err(invalid("xar TOC must have exactly one checksum element")),
        };

        let mut signatures = Vec::new();
        for element in toc
            .children()
            .filter(|n| n.has_tag_name("signature") || n.has_tag_name("x-signature"))
        {
            let offset = number(element, "offset")?;
            let size = number(element, "size")?;
            let bytes = read_range(&mut file, length, heap_start, offset, size)?;
            let certificates = element
                .descendants()
                .filter(|n| n.has_tag_name("X509Certificate"))
                .map(|n| decode_base64(n.text().unwrap_or_default()))
                .collect::<io::Result<_>>()?;
            reserved.push((offset, size));
            signatures.push(Signature {
                style: element.attribute("style").unwrap_or_default().to_owned(),
                bytes,
                certificates,
            });
        }
        if signatures.len() > 1
            && reserved
                .iter()
                .enumerate()
                .any(|(i, a)| reserved[i + 1..].iter().any(|b| overlaps(*a, *b)))
        {
            return Err(invalid("xar signature ranges overlap"));
        }

        let mut entries = Vec::new();
        files(toc, Path::new(""), &mut entries, &mut HashSet::new())?;
        for entry in &entries {
            if let Some(data) = &entry.data {
                let end = heap_start
                    .checked_add(data.offset)
                    .and_then(|v| v.checked_add(data.length))
                    .ok_or_else(|| invalid("xar data range overflows"))?;
                if end > length {
                    return Err(invalid(format!(
                        "xar data for {} is out of range",
                        entry.path.display()
                    )));
                }
                if reserved
                    .iter()
                    .any(|r| overlaps(*r, (data.offset, data.length)))
                {
                    return Err(invalid(format!(
                        "xar data for {} overlaps the TOC checksum or a signature",
                        entry.path.display()
                    )));
                }
            }
        }
        Ok(Self {
            file,
            heap_start,
            toc_xml,
            checksum,
            entries,
            signatures,
        })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The decompressed TOC.
    pub fn toc_xml(&self) -> &str {
        &self.toc_xml
    }

    /// The verified TOC checksum, which signatures cover.
    pub fn checksum(&self) -> Option<&(Algorithm, Vec<u8>)> {
        self.checksum.as_ref()
    }

    pub fn signatures(&self) -> &[Signature] {
        &self.signatures
    }

    /// Finds an entry by its path, such as `Distribution` or
    /// `Foo.pkg/PackageInfo`.
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == Path::new(path))
    }

    /// Decodes an entry's data into `writer`, verifying its checksums and
    /// size, and returns the decoded byte count.
    pub fn read_to(&mut self, data: &Data, writer: impl Write) -> io::Result<u64> {
        self.file
            .seek(SeekFrom::Start(self.heap_start + data.offset))?;
        let mut stored = Vec::new();
        (&mut self.file)
            .take(data.length)
            .read_to_end(&mut stored)?;
        if stored.len() as u64 != data.length {
            return Err(invalid("xar data is truncated"));
        }
        if let Some((algorithm, expected)) = &data.archived_checksum {
            if &algorithm.digest(&stored) != expected {
                return Err(invalid("xar archived checksum doesn't match"));
            }
        }
        let mut out = HashingWriter {
            inner: writer,
            hasher: data.extracted_checksum.as_ref().map(|(a, _)| a.hasher()),
        };
        let limit = data.size + 1;
        let written = match data.encoding {
            Encoding::None => io::copy(&mut stored.as_slice().take(limit), &mut out)?,
            Encoding::Zlib => io::copy(
                &mut flate2::read::ZlibDecoder::new(stored.as_slice()).take(limit),
                &mut out,
            )?,
            Encoding::Bzip2 => io::copy(
                &mut bzip2::read::BzDecoder::new(stored.as_slice()).take(limit),
                &mut out,
            )?,
            Encoding::Xz => io::copy(
                &mut xz2::read::XzDecoder::new(stored.as_slice()).take(limit),
                &mut out,
            )?,
        };
        if written != data.size {
            return Err(invalid("xar data size doesn't match its TOC"));
        }
        if let (Some((_, expected)), Some(hasher)) = (&data.extracted_checksum, out.hasher) {
            if &hasher.finish() != expected {
                return Err(invalid("xar extracted checksum doesn't match"));
            }
        }
        Ok(written)
    }

    /// Reads an entry's decoded data into memory, up to `limit` bytes.
    pub fn read(&mut self, path: &str, limit: u64) -> io::Result<Vec<u8>> {
        let data = self
            .entry(path)
            .and_then(|e| e.data.clone())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("xar archive has no {path}"),
                )
            })?;
        if data.size > limit {
            return Err(invalid(format!("xar entry {path} is too large")));
        }
        let mut bytes = Vec::with_capacity(data.size as usize);
        self.read_to(&data, &mut bytes)?;
        Ok(bytes)
    }

    /// Extracts entries like `xar -x -C destination`, applying TOC modes.
    /// Entries for which `skip` returns true aren't extracted. Ownership and
    /// extended attributes in the TOC aren't restored.
    pub fn extract(
        &mut self,
        destination: &Path,
        limits: Limits,
        skip: impl Fn(&Entry) -> bool,
    ) -> io::Result<Vec<SkippedXattr>> {
        let mut writer = TreeWriter::open(destination, limits)?;
        let entries: Vec<Entry> = self.entries.iter().filter(|e| !skip(e)).cloned().collect();
        let mut originals: HashMap<String, PathBuf> = HashMap::new();
        let mut links = Vec::new();
        for entry in &entries {
            match entry.kind {
                EntryKind::Directory => {
                    writer.create_dir(&entry.path, Some(entry.mode.unwrap_or(0o755)))?
                }
                EntryKind::Symlink => writer.symlink(
                    &entry.path,
                    std::ffi::OsStr::new(entry.link.as_deref().unwrap_or_default()),
                )?,
                EntryKind::HardLink if entry.link.as_deref() != Some("original") => {
                    links.push(entry.clone());
                }
                EntryKind::File | EntryKind::HardLink => {
                    let mode = entry.mode.unwrap_or(0o644);
                    match entry.data.clone() {
                        Some(data) => {
                            writer.write_file_with(&entry.path, mode, |sink| {
                                self.read_to(&data, sink).map(|_| ())
                            })?;
                        }
                        None => {
                            writer.write_file(&entry.path, io::empty(), mode)?;
                        }
                    }
                    originals.insert(entry.id.clone(), entry.path.clone());
                }
                EntryKind::Other => {}
            }
        }
        for link in links {
            let original = link
                .link
                .as_ref()
                .and_then(|id| originals.get(id))
                .ok_or_else(|| {
                    invalid(format!(
                        "xar hard link {} has no original",
                        link.path.display()
                    ))
                })?;
            writer.hard_link(&link.path, original)?;
        }
        writer.finish()
    }
}

fn read_range(
    file: &mut File,
    length: u64,
    heap_start: u64,
    offset: u64,
    size: u64,
) -> io::Result<Vec<u8>> {
    let start = heap_start
        .checked_add(offset)
        .ok_or_else(|| invalid("xar heap range overflows"))?;
    if start.checked_add(size).is_none_or(|end| end > length) || size > MAX_TOC_COMPRESSED {
        return Err(invalid("xar heap range is out of bounds"));
    }
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0u8; size as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn decode_base64(text: &str) -> io::Result<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
    {
        let value = TABLE
            .iter()
            .position(|t| *t == byte)
            .ok_or_else(|| invalid("xar certificate isn't valid base64"))?
            as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Ok(out)
}
