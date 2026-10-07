use crate::digest::Algorithm;
use crate::Encoding;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Where a file's contents come from.
pub enum Content {
    Bytes(Vec<u8>),
    /// A file read when the archive is written, so large payloads aren't
    /// held in memory.
    Path(PathBuf),
}

struct Node {
    name: String,
    mode: u32,
    /// `None` for a directory.
    content: Option<(Content, Encoding)>,
    children: Vec<Node>,
}

/// Builds a xar archive with a SHA-1 TOC checksum and SHA-1 file checksums,
/// the layout `pkgbuild` and `pkgutil --flatten` write.
pub struct Builder {
    root: Vec<Node>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Builder {
    pub fn new() -> Self {
        Self { root: Vec::new() }
    }

    fn slot(&mut self, path: &Path) -> io::Result<(&mut Vec<Node>, String)> {
        let parts: Vec<String> = path
            .components()
            .map(|c| match c {
                std::path::Component::Normal(name) => name
                    .to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("xar names must be UTF-8")),
                _ => Err(invalid("xar paths must be relative names")),
            })
            .collect::<io::Result<_>>()?;
        let (name, parents) = parts
            .split_last()
            .ok_or_else(|| invalid("empty xar path"))?;
        let mut level = &mut self.root;
        for parent in parents {
            let index = level
                .iter()
                .position(|n| &n.name == parent && n.content.is_none())
                .ok_or_else(|| invalid(format!("xar parent {parent} isn't a directory")))?;
            level = &mut level[index].children;
        }
        if level.iter().any(|n| &n.name == name) {
            return Err(invalid(format!(
                "xar path {} is already present",
                path.display()
            )));
        }
        Ok((level, name.clone()))
    }

    /// Adds a directory. Its parent must already be present.
    pub fn add_directory(&mut self, path: &Path, mode: u32) -> io::Result<()> {
        let (level, name) = self.slot(path)?;
        level.push(Node {
            name,
            mode,
            content: None,
            children: Vec::new(),
        });
        Ok(())
    }

    /// Adds a file, stored with `encoding`. [`Encoding::Xz`] isn't
    /// supported for writing.
    pub fn add_file(
        &mut self,
        path: &Path,
        mode: u32,
        content: Content,
        encoding: Encoding,
    ) -> io::Result<()> {
        if encoding == Encoding::Xz {
            return Err(invalid("xar xz encoding isn't supported for writing"));
        }
        let (level, name) = self.slot(path)?;
        level.push(Node {
            name,
            mode,
            content: Some((content, encoding)),
            children: Vec::new(),
        });
        Ok(())
    }

    /// Writes the archive to `out`.
    pub fn write(self, out: &Path) -> io::Result<()> {
        let directory = out
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut heap = tempfile::tempfile_in(directory)?;
        // The TOC checksum occupies the first 20 bytes of the heap.
        heap.write_all(&[0u8; 20])?;
        let mut toc = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xar>\n <toc>\n  <checksum style=\"sha1\">\n   <size>20</size>\n   <offset>0</offset>\n  </checksum>\n",
        );
        let mut next_id = 1;
        write_nodes(&self.root, &mut heap, &mut toc, &mut next_id, 2)?;
        toc.push_str(" </toc>\n</xar>\n");

        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(toc.as_bytes())?;
        let compressed = encoder.finish()?;
        let checksum = Algorithm::Sha1.digest(&compressed);

        let mut output = BufWriter::new(File::create(out)?);
        output.write_all(b"xar!")?;
        output.write_all(&28u16.to_be_bytes())?;
        output.write_all(&1u16.to_be_bytes())?;
        output.write_all(&(compressed.len() as u64).to_be_bytes())?;
        output.write_all(&(toc.len() as u64).to_be_bytes())?;
        output.write_all(&1u32.to_be_bytes())?;
        output.write_all(&compressed)?;
        output.write_all(&checksum)?;
        heap.seek(SeekFrom::Start(20))?;
        io::copy(&mut heap, &mut output)?;
        output.flush()
    }
}

fn write_nodes(
    nodes: &[Node],
    heap: &mut File,
    toc: &mut String,
    next_id: &mut u32,
    depth: usize,
) -> io::Result<()> {
    let pad = " ".repeat(depth);
    for node in nodes {
        let id = *next_id;
        *next_id += 1;
        toc.push_str(&format!("{pad}<file id=\"{id}\">\n"));
        toc.push_str(&format!("{pad} <name>{}</name>\n", escape(&node.name)));
        match &node.content {
            None => {
                toc.push_str(&format!("{pad} <type>directory</type>\n"));
                toc.push_str(&format!("{pad} <mode>{:04o}</mode>\n", node.mode & 0o7777));
                write_nodes(&node.children, heap, toc, next_id, depth + 1)?;
            }
            Some((content, encoding)) => {
                toc.push_str(&format!("{pad} <type>file</type>\n"));
                toc.push_str(&format!("{pad} <mode>{:04o}</mode>\n", node.mode & 0o7777));
                let offset = heap.stream_position()?;
                let mut reader: Box<dyn Read> = match content {
                    Content::Bytes(bytes) => Box::new(io::Cursor::new(bytes.clone())),
                    Content::Path(path) => Box::new(io::BufReader::new(File::open(path)?)),
                };
                let mut extracted = Algorithm::Sha1.hasher();
                let mut archived = Algorithm::Sha1.hasher();
                let mut size = 0u64;
                let mut buffer = vec![0u8; 1 << 16];
                let length = if *encoding == Encoding::None {
                    loop {
                        let read = reader.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }
                        extracted.update(&buffer[..read]);
                        archived.update(&buffer[..read]);
                        heap.write_all(&buffer[..read])?;
                        size += read as u64;
                    }
                    size
                } else {
                    let mut plain = Vec::new();
                    loop {
                        let read = reader.read(&mut buffer)?;
                        if read == 0 {
                            break;
                        }
                        extracted.update(&buffer[..read]);
                        size += read as u64;
                        plain.extend_from_slice(&buffer[..read]);
                    }
                    let stored = compress(&plain, *encoding)?;
                    archived.update(&stored);
                    heap.write_all(&stored)?;
                    stored.len() as u64
                };
                let encoding = match encoding {
                    Encoding::None => "application/octet-stream",
                    Encoding::Zlib => "application/x-gzip",
                    _ => "application/x-bzip2",
                };
                toc.push_str(&format!(
                    "{pad} <data>\n{pad}  <length>{length}</length>\n{pad}  <offset>{}</offset>\n{pad}  <size>{size}</size>\n{pad}  <encoding style=\"{encoding}\"/>\n{pad}  <archived-checksum style=\"sha1\">{}</archived-checksum>\n{pad}  <extracted-checksum style=\"sha1\">{}</extracted-checksum>\n{pad} </data>\n",
                    offset,
                    hex(&archived.finish()),
                    hex(&extracted.finish()),
                ));
            }
        }
        toc.push_str(&format!("{pad}</file>\n"));
    }
    Ok(())
}

fn compress(plain: &[u8], encoding: Encoding) -> io::Result<Vec<u8>> {
    if encoding == Encoding::Bzip2 {
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
        encoder.write_all(plain)?;
        encoder.finish()
    } else {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain)?;
        encoder.finish()
    }
}
