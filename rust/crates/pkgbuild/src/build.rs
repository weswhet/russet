use crate::analyze::Component;
use russet_xar::{Builder, Content, Encoding};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Names `pkgbuild` leaves out of every payload.
const EXCLUDED: [&str; 4] = [".DS_Store", ".svn", "CVS", ".git"];

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// What a payload path is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Directory,
    /// A regular file, read from this path when the package is built.
    File(PathBuf),
    Symlink(String),
}

/// One payload path with the ownership and mode the package records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Path relative to the root, without `./`; empty for the root.
    pub path: String,
    pub kind: NodeKind,
    /// Permission bits, including set-ID and sticky bits.
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u32,
}

/// Lists `root` in sorted, depth-first order, without following symlinks
/// and without the names `pkgbuild` always excludes or paths matching a
/// filter. Ownership and modes come from the disk; callers adjust them.
pub fn collect(root: &Path, filters: &[regex::Regex]) -> io::Result<Vec<Node>> {
    let mut nodes = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(relative) = stack.pop() {
        let path = if relative.is_empty() {
            root.to_path_buf()
        } else {
            root.join(&relative)
        };
        let metadata = fs::symlink_metadata(&path)?;
        let kind = if metadata.file_type().is_symlink() {
            NodeKind::Symlink(
                fs::read_link(&path)?
                    .to_str()
                    .ok_or_else(|| {
                        invalid(format!("{} has a target that isn't UTF-8", path.display()))
                    })?
                    .to_owned(),
            )
        } else if metadata.is_dir() {
            let mut names = Vec::new();
            for entry in fs::read_dir(&path)? {
                let name = entry?.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| invalid(format!("A name in {} isn't UTF-8", path.display())))?
                    .to_owned();
                let child = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}/{name}")
                };
                if EXCLUDED.contains(&name.as_str()) || filters.iter().any(|f| f.is_match(&child)) {
                    continue;
                }
                names.push(child);
            }
            names.sort();
            stack.extend(names.into_iter().rev());
            NodeKind::Directory
        } else if metadata.is_file() {
            NodeKind::File(path.clone())
        } else {
            continue;
        };
        nodes.push(Node {
            path: relative,
            kind,
            mode: (metadata.permissions().mode() & 0o7777) as u16,
            uid: metadata.uid(),
            gid: metadata.gid(),
            mtime: u32::try_from(metadata.mtime().max(0)).unwrap_or(u32::MAX),
        });
    }
    Ok(nodes)
}

/// How to build the package.
pub struct Options<'a> {
    pub identifier: &'a str,
    pub version: &'a str,
    /// `--install-location`.
    pub install_location: Option<&'a str>,
    /// `--min-os-version`.
    pub min_os_version: Option<&'a str>,
    /// `--scripts`: a folder with `preinstall` and `postinstall`.
    pub scripts: Option<&'a Path>,
    /// `--info`: a PackageInfo template.
    pub info_template: Option<&'a Path>,
    /// The bundles from [`crate::analyze`].
    pub components: &'a [Component],
}

/// Counts bytes passing through, for the payload size.
struct Counting<W> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buf)?;
        self.count += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn type_bits(kind: &NodeKind) -> u32 {
    match kind {
        NodeKind::Directory => 0o040000,
        NodeKind::File(_) => 0o100000,
        NodeKind::Symlink(_) => 0o120000,
    }
}

/// Writes the payload and returns its BOM entries and uncompressed size.
fn payload(nodes: &[Node], out: &Path) -> io::Result<(Vec<russet_mkbom::Entry>, u64)> {
    let file = fs::File::create(out)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut writer = russet_ditto::CpioWriter::new(Counting {
        inner: encoder,
        count: 0,
    });
    let mut entries = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let name = if node.path.is_empty() {
            ".".to_owned()
        } else {
            format!("./{}", node.path)
        };
        let header = russet_ditto::Header {
            mode: type_bits(&node.kind) | u32::from(node.mode),
            uid: node.uid,
            gid: node.gid,
            mtime: u64::from(node.mtime),
            ino: index as u64 + 1,
            nlink: 1,
        };
        let kind = match &node.kind {
            NodeKind::Directory => {
                writer.append(&name, header, io::empty(), 0)?;
                russet_mkbom::Kind::Directory
            }
            NodeKind::Symlink(target) => {
                writer.append(&name, header, target.as_bytes(), target.len() as u64)?;
                russet_mkbom::Kind::Symlink {
                    target: target.clone(),
                    checksum: russet_mkbom::cksum(target.as_bytes()),
                }
            }
            NodeKind::File(source) => {
                // Summarize the file first, then copy it, so neither step
                // holds it in memory.
                let summary = russet_mkbom::scan(fs::File::open(source)?, io::sink())?;
                writer.append(&name, header, fs::File::open(source)?, summary.size)?;
                russet_mkbom::Kind::File {
                    size: summary.size,
                    checksum: summary.checksum,
                    archs: summary.archs,
                }
            }
        };
        entries.push(russet_mkbom::Entry {
            path: node.path.clone(),
            kind,
            mode: node.mode,
            uid: node.uid,
            gid: node.gid,
            mtime: node.mtime,
        });
    }
    let counting = writer.finish()?;
    let size = counting.count;
    counting.inner.finish()?.flush()?;
    Ok((entries, size))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Writes an element from a parsed template, with its children.
fn serialize(node: roxmltree::Node, depth: usize, out: &mut String) {
    let pad = "    ".repeat(depth);
    let name = node.tag_name().name();
    out.push_str(&format!("{pad}<{name}"));
    for attribute in node.attributes() {
        out.push_str(&format!(
            " {}=\"{}\"",
            attribute.name(),
            escape(attribute.value())
        ));
    }
    let children: Vec<_> = node.children().filter(|c| c.is_element()).collect();
    let text = node.text().map(str::trim).filter(|t| !t.is_empty());
    if children.is_empty() && text.is_none() {
        out.push_str("/>\n");
        return;
    }
    out.push('>');
    if let Some(text) = text {
        out.push_str(&escape(text));
    }
    if !children.is_empty() {
        out.push('\n');
        for child in children {
            serialize(child, depth + 1, out);
        }
        out.push_str(&pad);
    }
    out.push_str(&format!("</{name}>\n"));
}

/// The identifiers of the bundles in `source` that pass `test`.
fn ids<'a>(source: &[&'a Component], test: impl Fn(&Component) -> bool) -> Vec<&'a str> {
    source
        .iter()
        .filter(|c| test(c))
        .map(|c| c.identifier.as_str())
        .collect()
}

fn bundle_list(out: &mut String, tag: &str, ids: &[&str]) {
    if ids.is_empty() {
        out.push_str(&format!("    <{tag}/>\n"));
    } else {
        out.push_str(&format!("    <{tag}>\n"));
        for id in ids {
            out.push_str(&format!("        <bundle id=\"{}\"/>\n", escape(id)));
        }
        out.push_str(&format!("    </{tag}>\n"));
    }
}

fn package_info(options: &Options, files: usize, kilobytes: u64) -> io::Result<String> {
    let template = options.info_template.map(fs::read_to_string).transpose()?;
    let document = template
        .as_deref()
        .map(roxmltree::Document::parse)
        .transpose()
        .map_err(|e| invalid(format!("The PackageInfo template isn't valid XML: {e}")))?;
    let root = document
        .as_ref()
        .map(|d| d.root_element())
        .filter(|r| r.has_tag_name("pkg-info"));
    if document.is_some() && root.is_none() {
        return Err(invalid(
            "The PackageInfo template's root element isn't pkg-info",
        ));
    }
    let mut attributes: Vec<(String, String)> = match root {
        Some(root) => root
            .attributes()
            .filter(|a| {
                ![
                    "identifier",
                    "version",
                    "auth",
                    "format-version",
                    "generator-version",
                ]
                .contains(&a.name())
            })
            .map(|a| (a.name().to_owned(), a.value().to_owned()))
            .collect(),
        None => vec![
            ("overwrite-permissions".into(), "true".into()),
            ("relocatable".into(), "false".into()),
            ("postinstall-action".into(), "none".into()),
        ],
    };
    let mut set = |key: &str, value: &str| {
        attributes.retain(|(k, _)| k != key);
        attributes.push((key.to_owned(), value.to_owned()));
    };
    set("identifier", options.identifier);
    set("version", options.version);
    set("format-version", "2");
    set("generator-version", "Russet");
    if let Some(location) = options.install_location {
        set("install-location", location);
    }
    set("auth", "root");
    if let Some(version) = options.min_os_version {
        set("minimumSystemVersion", version);
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<pkg-info");
    for (key, value) in &attributes {
        out.push_str(&format!(" {key}=\"{}\"", escape(value)));
    }
    out.push_str(">\n");
    if let Some(root) = root {
        for child in root.children().filter(|c| c.is_element()) {
            serialize(child, 1, &mut out);
        }
    }
    out.push_str(&format!(
        "    <payload numberOfFiles=\"{files}\" installKBytes=\"{kilobytes}\"/>\n"
    ));
    let all: Vec<&Component> = options
        .components
        .iter()
        .chain(options.components.iter().flat_map(|c| c.children.iter()))
        .collect();
    for bundle in &all {
        out.push_str(&format!(
            "    <bundle path=\"./{}\" id=\"{}\"",
            escape(&bundle.path),
            escape(&bundle.identifier)
        ));
        if let Some(version) = &bundle.short_version {
            out.push_str(&format!(
                " CFBundleShortVersionString=\"{}\"",
                escape(version)
            ));
        }
        if let Some(version) = &bundle.version {
            out.push_str(&format!(" CFBundleVersion=\"{}\"", escape(version)));
        }
        out.push_str("/>\n");
    }
    let top: Vec<&Component> = options.components.iter().collect();
    bundle_list(
        &mut out,
        "bundle-version",
        &ids(&top, |c| c.version_checked),
    );
    bundle_list(
        &mut out,
        "upgrade-bundle",
        &ids(&top, |c| c.overwrite_action == "upgrade"),
    );
    bundle_list(
        &mut out,
        "update-bundle",
        &ids(&top, |c| c.overwrite_action == "update"),
    );
    bundle_list(
        &mut out,
        "atomic-update-bundle",
        &ids(&top, |c| c.overwrite_action == "atomic"),
    );
    bundle_list(
        &mut out,
        "strict-identifier",
        &ids(&all, |c| c.strict_identifier),
    );
    bundle_list(&mut out, "relocate", &ids(&top, |c| c.relocatable));
    if let Some(scripts) = options.scripts {
        let present: Vec<&str> = ["preinstall", "postinstall"]
            .into_iter()
            .filter(|name| scripts.join(name).is_file())
            .collect();
        if !present.is_empty() {
            out.push_str("    <scripts>\n");
            for name in present {
                out.push_str(&format!(
                    "        <{name} file=\"./{name}\" timeout=\"600\"/>\n"
                ));
            }
            out.push_str("    </scripts>\n");
        }
    }
    out.push_str("</pkg-info>\n");
    Ok(out)
}

/// Builds a component package at `out` from `nodes`, which must start with
/// the root and list folders before their contents.
pub fn build(nodes: &[Node], options: &Options, out: &Path) -> io::Result<()> {
    if nodes
        .first()
        .is_none_or(|n| !n.path.is_empty() || n.kind != NodeKind::Directory)
    {
        return Err(invalid("The payload must start with its root folder"));
    }
    let directory = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let scratch = tempfile::tempdir_in(directory)?;
    let payload_path = scratch.path().join("Payload");
    let (entries, size) = payload(nodes, &payload_path)?;
    let bom = russet_mkbom::write(&entries).map_err(invalid)?;
    let mut builder = Builder::new();
    builder.add_file(Path::new("Bom"), 0o644, Content::Bytes(bom), Encoding::Zlib)?;
    builder.add_file(
        Path::new("Payload"),
        0o644,
        Content::Path(payload_path),
        Encoding::None,
    )?;
    if let Some(scripts) = options.scripts {
        let archive = scratch.path().join("Scripts");
        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&archive)?,
            flate2::Compression::default(),
        );
        russet_ditto::write_tree(scripts, encoder, |_, m| russet_ditto::Header {
            mode: m.mode(),
            uid: 0,
            gid: 0,
            mtime: m.mtime().max(0) as u64,
            ino: 0,
            nlink: 1,
        })?
        .finish()?
        .flush()?;
        builder.add_file(
            Path::new("Scripts"),
            0o644,
            Content::Path(archive),
            Encoding::None,
        )?;
    }
    let info = package_info(options, entries.len(), size.div_ceil(1024))?;
    builder.add_file(
        Path::new("PackageInfo"),
        0o644,
        Content::Bytes(info.into_bytes()),
        Encoding::Zlib,
    )?;
    builder.write(out)
}
