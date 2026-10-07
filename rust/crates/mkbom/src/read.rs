use crate::{Arch, Entry, Kind};
use std::collections::HashMap;

fn invalid(message: &str) -> String {
    format!("Invalid BOM: {message}")
}

struct Bom<'a> {
    bytes: &'a [u8],
    index: usize,
    count: u32,
}

fn be32(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid("truncated"))
}

fn be16(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
        .ok_or_else(|| invalid("truncated"))
}

impl<'a> Bom<'a> {
    fn block(&self, id: u32) -> Result<&'a [u8], String> {
        if id >= self.count {
            return Err(invalid("block index out of range"));
        }
        let at = self.index + 4 + id as usize * 8;
        let offset = be32(self.bytes, at)? as usize;
        let length = be32(self.bytes, at + 4)? as usize;
        self.bytes
            .get(
                offset
                    ..offset
                        .checked_add(length)
                        .ok_or_else(|| invalid("bad block"))?,
            )
            .ok_or_else(|| invalid("block out of range"))
    }

    fn var(&self, name: &str) -> Result<u32, String> {
        let offset = be32(self.bytes, 24)? as usize;
        let count = be32(self.bytes, offset)?;
        let mut at = offset + 4;
        for _ in 0..count.min(64) {
            let block = be32(self.bytes, at)?;
            let length = *self
                .bytes
                .get(at + 4)
                .ok_or_else(|| invalid("truncated vars"))? as usize;
            let var = self
                .bytes
                .get(at + 5..at + 5 + length)
                .ok_or_else(|| invalid("truncated vars"))?;
            if var == name.as_bytes() {
                return Ok(block);
            }
            at += 5 + length;
        }
        Err(invalid(&format!("no {name} variable")))
    }
}

fn c_string(bytes: &[u8]) -> Result<String, String> {
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| invalid("unterminated name"))?;
    String::from_utf8(bytes[..end].to_vec()).map_err(|_| invalid("name isn't UTF-8"))
}

fn record(bytes: &[u8]) -> Result<(Kind, u16, u32, u32, u32), String> {
    let kind = *bytes.first().ok_or_else(|| invalid("empty record"))?;
    let arch = be16(bytes, 2)?;
    let mode = be16(bytes, 4)? & 0o7777;
    let uid = be32(bytes, 6)?;
    let gid = be32(bytes, 10)?;
    let mtime = be32(bytes, 14)?;
    let size = be32(bytes, 18)?;
    let checksum = be32(bytes, 23)?;
    let kind = match kind {
        2 => Kind::Directory,
        1 => {
            let mut archs = Vec::new();
            if arch & 0x2000 != 0 && bytes.get(27) == Some(&1) {
                let count = be32(bytes, 28)?;
                for i in 0..count.min(64) as usize {
                    let at = 32 + i * 16;
                    archs.push(Arch {
                        cpu_type: be32(bytes, at)?,
                        cpu_subtype: be32(bytes, at + 4)?,
                        size: be32(bytes, at + 8)?,
                        checksum: be32(bytes, at + 12)?,
                    });
                }
            }
            Kind::File {
                size: u64::from(size),
                checksum,
                archs,
            }
        }
        3 => {
            let length = be32(bytes, 27)? as usize;
            let target = bytes
                .get(31..31 + length)
                .ok_or_else(|| invalid("truncated link"))?;
            Kind::Symlink {
                target: c_string(target)?,
                checksum,
            }
        }
        other => return Err(invalid(&format!("unsupported path type {other}"))),
    };
    Ok((kind, mode, uid, gid, mtime))
}

/// The sizes of files of 4 GiB or more, by record block, from the Size64
/// tree. Each leaf entry's key block holds a record's block number and its
/// value block the 64-bit size.
fn sizes_64(bom: &Bom) -> Result<HashMap<u32, u64>, String> {
    let mut out = HashMap::new();
    let Ok(var) = bom.var("Size64") else {
        return Ok(out);
    };
    let tree = bom.block(var)?;
    if tree.get(..4) != Some(b"tree") {
        return Err(invalid("Size64 isn't a tree"));
    }
    let mut node = be32(tree, 8)?;
    let mut visited = 0;
    while node != 0 {
        visited += 1;
        if visited > bom.count {
            return Err(invalid("Size64 links form a cycle"));
        }
        let block = bom.block(node)?;
        if be16(block, 0)? != 1 {
            return Err(invalid("Size64 has more than one level"));
        }
        for i in 0..be16(block, 2)? as usize {
            let value = bom.block(be32(block, 12 + i * 8)?)?;
            let key = bom.block(be32(block, 16 + i * 8)?)?;
            let size = value
                .get(..8)
                .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
                .ok_or_else(|| invalid("truncated Size64 value"))?;
            out.insert(be32(key, 0)?, size);
        }
        node = be32(block, 4)?;
    }
    Ok(out)
}

/// Reads every path in a BOM, in the order `lsbom` lists them.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.get(..8) != Some(b"BOMStore") {
        return Err(invalid("missing BOMStore magic"));
    }
    let index = be32(bytes, 16)? as usize;
    let bom = Bom {
        bytes,
        index,
        count: be32(bytes, index)?,
    };
    let tree = bom.block(bom.var("Paths")?)?;
    if tree.get(..4) != Some(b"tree") {
        return Err(invalid("Paths isn't a tree"));
    }
    // Descend to the first leaf, then follow the forward links.
    let mut node = be32(tree, 8)?;
    for _ in 0..16 {
        let block = bom.block(node)?;
        if be16(block, 0)? == 1 {
            break;
        }
        node = be32(block, 12)?;
    }
    let large = sizes_64(&bom)?;
    let mut paths: HashMap<u32, String> = HashMap::new();
    let mut entries = Vec::new();
    let mut visited = 0;
    while node != 0 {
        visited += 1;
        if visited > bom.count {
            return Err(invalid("leaf links form a cycle"));
        }
        let block = bom.block(node)?;
        if be16(block, 0)? != 1 {
            return Err(invalid("expected a leaf node"));
        }
        let count = be16(block, 2)? as usize;
        for i in 0..count {
            let info = bom.block(be32(block, 12 + i * 8)?)?;
            let name = bom.block(be32(block, 16 + i * 8)?)?;
            let id = be32(info, 0)?;
            let parent = be32(name, 0)?;
            let leaf = c_string(&name[4..])?;
            let path = match parent {
                0 => String::new(),
                1 => leaf,
                parent => format!(
                    "{}/{leaf}",
                    paths
                        .get(&parent)
                        .ok_or_else(|| invalid("a path's parent comes after it"))?
                ),
            };
            let record_block = be32(info, 4)?;
            let (mut kind, mode, uid, gid, mtime) = record(bom.block(record_block)?)?;
            if let (Kind::File { size, .. }, Some(full)) = (&mut kind, large.get(&record_block)) {
                *size = *full;
            }
            paths.insert(id, path.clone());
            entries.push(Entry {
                path,
                kind,
                mode,
                uid,
                gid,
                mtime,
            });
        }
        node = be32(block, 4)?;
    }
    Ok(entries)
}
