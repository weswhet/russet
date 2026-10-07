# mkbom and lsbom

Russet's `russet-mkbom` crate writes and reads BOM files on Linux. These
notes record the format details it matches, observed with `mkbom` and `lsbom`
on macOS 27.0 (build 26A428). The tests in `rust/crates/mkbom` check, on every
macOS CI runner, that `lsbom` lists native BOMs exactly as it lists `mkbom`'s,
including architecture details, and that the native reader lists `mkbom`'s
BOMs the same way.

- Checksums are POSIX `cksum` CRCs (CRC-32 over the data, then the length),
  not zlib CRC-32.
- Path records: type (1 file, 2 folder, 3 symlink), architecture flags
  (`0x000f`, or `0x200f` when a Mach-O architecture list follows), mode, uid,
  gid, mtime, size, and checksum. Folders end after a zero link length. Files
  add an optional architecture list (CPU type, subtype, slice size, slice
  checksum) and 8 zero bytes. Symlinks record the target's length and
  checksum, then the target, then 8 zero bytes.
- `BomInfo` totals bytes per CPU type, with an entry for CPU type 0 (files that
  aren't Mach-O) first.
- `Paths` is a B+ tree of 4 KiB nodes. Leaves hold (index block, name block)
  pairs in path-ID order, a sorted depth-first walk, and are linked forward
  and backward. Index nodes point to leaves by their last key. Names are
  stored without their parent paths, keyed by the parent's path ID.
- `mkbom` also writes a small tree per path and empty `HLIndex`, `VIndex`,
  and `Size64` trees; Russet writes them for parity.
- Files of 4 GiB or more: the path record's 32-bit size holds the low 32
  bits, and the `Size64` tree has a leaf entry whose key block holds the
  record's block number and whose value block holds the full size as a
  big-endian 64-bit number. The `BomInfo` totals keep only their low 32
  bits.
