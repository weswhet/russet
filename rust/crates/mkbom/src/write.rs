use crate::{Entry, Kind};
use std::collections::{BTreeMap, HashMap};

const TREE_BLOCK_SIZE: u32 = 4096;
const VINDEX_BLOCK_SIZE: u32 = 128;
const PATH_TREE_BLOCK_SIZE: u32 = 64;
/// Leaf entries per Paths node. A node holds up to 510; `mkbom` leaves
/// room, and so does this.
const ENTRIES_PER_LEAF: usize = 300;
/// Where block data starts, leaving room for the header and variables.
const BLOCK_DATA_OFFSET: usize = 512;
const VARS_OFFSET: usize = 32;

fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn tree(child: u32, block_size: u32, count: u32) -> Vec<u8> {
    let mut out = b"tree".to_vec();
    be32(&mut out, 1);
    be32(&mut out, child);
    be32(&mut out, block_size);
    be32(&mut out, count);
    out.push(0);
    out
}

/// A Paths node: leaf flag, entry count, forward and backward links, then
/// (value block, key block) pairs, padded to the tree's block size.
fn paths_node(
    leaf: bool,
    entries: &[(u32, u32)],
    forward: u32,
    backward: u32,
    size: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(size as usize);
    be16(&mut out, u16::from(leaf));
    be16(&mut out, entries.len() as u16);
    be32(&mut out, forward);
    be32(&mut out, backward);
    for (value, key) in entries {
        be32(&mut out, *value);
        be32(&mut out, *key);
    }
    out.resize(size as usize, 0);
    out
}

fn record(entry: &Entry) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let (kind, archs) = match &entry.kind {
        Kind::Directory => (2u8, &[][..]),
        Kind::File { archs, .. } => (1, archs.as_slice()),
        Kind::Symlink { .. } => (3, &[][..]),
    };
    out.push(kind);
    out.push(1);
    be16(&mut out, if archs.is_empty() { 0x000f } else { 0x200f });
    be16(&mut out, entry.full_mode());
    be32(&mut out, entry.uid);
    be32(&mut out, entry.gid);
    be32(&mut out, entry.mtime);
    let (size, checksum) = match &entry.kind {
        Kind::Directory => (0, 0),
        Kind::File { size, checksum, .. } => (*size, *checksum),
        Kind::Symlink { target, checksum } => (target.len() as u64, *checksum),
    };
    // Sizes of 4 GiB or more keep their low 32 bits here; the Size64 tree
    // holds the whole size.
    be32(&mut out, size as u32);
    out.push(1);
    be32(&mut out, checksum);
    match &entry.kind {
        Kind::Directory => be32(&mut out, 0),
        Kind::File { .. } => {
            if !archs.is_empty() {
                out.push(1);
                be32(&mut out, archs.len() as u32);
                for arch in archs {
                    be32(&mut out, arch.cpu_type);
                    be32(&mut out, arch.cpu_subtype);
                    be32(&mut out, arch.size);
                    be32(&mut out, arch.checksum);
                }
            }
            be32(&mut out, 0);
            be32(&mut out, 0);
        }
        Kind::Symlink { target, .. } => {
            be32(&mut out, target.len() as u32 + 1);
            out.extend_from_slice(target.as_bytes());
            out.push(0);
            be32(&mut out, 0);
            be32(&mut out, 0);
        }
    }
    Ok(out)
}

/// Writes a BOM for `entries`, which must start with the root (an empty
/// path) and list every directory before its contents, in the order the BOM
/// should list them (normally sorted, depth first).
pub fn write(entries: &[Entry]) -> Result<Vec<u8>, String> {
    if entries
        .first()
        .is_none_or(|e| !e.path.is_empty() || e.kind != Kind::Directory)
    {
        return Err("A BOM must start with its root directory".into());
    }
    let mut blocks: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let mut leaf_entries = Vec::with_capacity(entries.len());
    let mut records = Vec::with_capacity(entries.len());
    let mut totals: BTreeMap<u32, u64> = BTreeMap::from([(0, 0)]);
    let mut large: Vec<(u32, u32)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let id = index as u32 + 1;
        let (parent, name) = if entry.path.is_empty() {
            (0, ".")
        } else {
            match entry.path.rsplit_once('/') {
                Some((parent, name)) => (
                    *ids.get(parent)
                        .ok_or_else(|| format!("{}'s parent isn't listed before it", entry.path))?,
                    name,
                ),
                None => (1, entry.path.as_str()),
            }
        };
        if name.is_empty() || name.contains('\0') {
            return Err(format!("Invalid BOM path {:?}", entry.path));
        }
        if ids.insert(&entry.path, id).is_some() {
            return Err(format!("{} is listed twice", entry.path));
        }
        if let Kind::File { size, archs, .. } = &entry.kind {
            if archs.is_empty() {
                *totals.entry(0).or_default() += size;
            }
            for arch in archs {
                *totals.entry(arch.cpu_type).or_default() += u64::from(arch.size);
            }
        }
        let record_block = blocks.len() as u32;
        blocks.push(record(entry)?);
        records.push(record_block);
        if let Kind::File { size, .. } = &entry.kind {
            if *size > u64::from(u32::MAX) {
                // A Size64 entry: the record's block number, then the size.
                let key = blocks.len() as u32;
                blocks.push(record_block.to_be_bytes().to_vec());
                blocks.push(size.to_be_bytes().to_vec());
                large.push((key + 1, key));
            }
        }
        let name_block = blocks.len() as u32;
        let mut name_bytes = Vec::with_capacity(name.len() + 5);
        be32(&mut name_bytes, parent);
        name_bytes.extend_from_slice(name.as_bytes());
        name_bytes.push(0);
        blocks.push(name_bytes);
        let info_block = blocks.len() as u32;
        let mut info = Vec::with_capacity(8);
        be32(&mut info, id);
        be32(&mut info, record_block);
        blocks.push(info);
        leaf_entries.push((info_block, name_block));
    }

    // BomInfo: path count (plus one), then bytes per CPU type.
    let mut info = Vec::new();
    be32(&mut info, 1);
    be32(&mut info, entries.len() as u32 + 1);
    be32(&mut info, totals.len() as u32);
    for (cpu, total) in &totals {
        be32(&mut info, *cpu);
        be32(&mut info, 0);
        // `mkbom` keeps only the low 32 bits of each total.
        be32(&mut info, *total as u32);
        be32(&mut info, 0);
    }
    blocks[1] = info;

    // `mkbom` keeps a small tree for each path; readers ignore them, but
    // they're written for parity.
    for record_block in records {
        let tree_block = blocks.len() as u32;
        blocks.push(tree(tree_block + 1, PATH_TREE_BLOCK_SIZE, 0));
        blocks.push(paths_node(true, &[], 0, 0, PATH_TREE_BLOCK_SIZE));
        blocks.push(record_block.to_be_bytes().to_vec());
        blocks.push(tree_block.to_be_bytes().to_vec());
    }

    // Paths: a tree whose leaves hold the entries in order, linked forward
    // and backward, under an index node when there's more than one leaf.
    let paths = blocks.len() as u32;
    blocks.push(Vec::new());
    let chunks: Vec<&[(u32, u32)]> = leaf_entries.chunks(ENTRIES_PER_LEAF).collect();
    let first_leaf = blocks.len() as u32 + u32::from(chunks.len() > 1);
    let root = if chunks.len() > 1 {
        let index: Vec<(u32, u32)> = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| (first_leaf + i as u32, chunk.last().unwrap().1))
            .collect();
        let root = blocks.len() as u32;
        blocks.push(paths_node(false, &index, 0, 0, TREE_BLOCK_SIZE));
        root
    } else {
        first_leaf
    };
    for (i, chunk) in chunks.iter().enumerate() {
        let block = first_leaf + i as u32;
        let forward = if i + 1 < chunks.len() { block + 1 } else { 0 };
        let backward = if i > 0 { block - 1 } else { 0 };
        blocks.push(paths_node(true, chunk, forward, backward, TREE_BLOCK_SIZE));
    }
    blocks[paths as usize] = tree(root, TREE_BLOCK_SIZE, entries.len() as u32);

    let empty_tree = |blocks: &mut Vec<Vec<u8>>, size: u32| -> u32 {
        let at = blocks.len() as u32;
        blocks.push(tree(at + 1, size, 0));
        blocks.push(paths_node(true, &[], 0, 0, size));
        at
    };
    let hard_links = empty_tree(&mut blocks, TREE_BLOCK_SIZE);
    let vindex = blocks.len() as u32;
    blocks.push(Vec::new());
    let vindex_tree = empty_tree(&mut blocks, VINDEX_BLOCK_SIZE);
    let mut vindex_block = Vec::new();
    be32(&mut vindex_block, 1);
    be32(&mut vindex_block, vindex_tree);
    be32(&mut vindex_block, 0);
    vindex_block.push(0);
    blocks[vindex as usize] = vindex_block;
    let size64 = if large.is_empty() {
        empty_tree(&mut blocks, TREE_BLOCK_SIZE)
    } else {
        if large.len() > ENTRIES_PER_LEAF {
            return Err(format!(
                "More than {ENTRIES_PER_LEAF} files are 4 GiB or larger, which isn't supported"
            ));
        }
        let at = blocks.len() as u32;
        blocks.push(tree(at + 1, TREE_BLOCK_SIZE, large.len() as u32));
        blocks.push(paths_node(true, &large, 0, 0, TREE_BLOCK_SIZE));
        at
    };

    let mut vars = Vec::new();
    be32(&mut vars, 5);
    for (block, name) in [
        (1, "BomInfo"),
        (paths, "Paths"),
        (hard_links, "HLIndex"),
        (vindex, "VIndex"),
        (size64, "Size64"),
    ] {
        be32(&mut vars, block);
        vars.push(name.len() as u8);
        vars.extend_from_slice(name.as_bytes());
    }

    let mut out = vec![0u8; BLOCK_DATA_OFFSET];
    let mut index = Vec::with_capacity(4 + blocks.len() * 8 + 20);
    be32(&mut index, blocks.len() as u32);
    for block in &blocks {
        let offset = if block.is_empty() {
            0
        } else {
            out.len() as u32
        };
        be32(&mut index, offset);
        be32(&mut index, block.len() as u32);
        out.extend_from_slice(block);
    }
    // An empty free list, as two null entries.
    be32(&mut index, 2);
    index.extend_from_slice(&[0; 16]);
    let index_offset = out.len();
    out.extend_from_slice(&index);

    let mut header = b"BOMStore".to_vec();
    be32(&mut header, 1);
    be32(&mut header, blocks.len() as u32 - 1);
    be32(&mut header, index_offset as u32);
    be32(&mut header, index.len() as u32);
    be32(&mut header, VARS_OFFSET as u32);
    be32(&mut header, vars.len() as u32);
    out[..header.len()].copy_from_slice(&header);
    out[VARS_OFFSET..VARS_OFFSET + vars.len()].copy_from_slice(&vars);
    Ok(out)
}
