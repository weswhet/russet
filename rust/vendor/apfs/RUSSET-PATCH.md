# Russet patch

This directory vendors apfs 0.4.1 from crates.io
(<https://github.com/Dil4rd/dpp>). Its MIT license is retained in `LICENSE`.

Russet's `russet-hdiutil` crate uses apfs to read APFS volumes on Linux. Each
change is marked `Russet patch` in the source:

- **Range scans descend the tree.** `btree::btree_scan` visited every child
  subtree from the left of each index node until a key ordered after the
  range, so finding one directory's records read every leaf before them.
  Path lookups use it for each component, so extracting a volume was
  quadratic in its file count, and every node read is checksummed: Electron
  apps with a few thousand files took longer than ten minutes. The scan now
  skips a child when the next child's first key still orders before the
  range, so it reads only the path to the range and the leaves in it. It
  returns the same records.
- **Access by object ID.** Every `ApfsVolume` method took a path, and a
  path lookup scans every record of each directory on the path, so a walk
  that named each entry by path paid for its directories again for every
  file. `root_oid`, `list_directory_by_oid`, `stat_by_oid`,
  `read_file_to_by_oid`, `open_file_by_oid`, `list_xattrs_by_oid`, and
  `get_xattr_by_oid` take the object ID that `DirEntry::oid` reports. The
  path methods resolve the path and call the same code.

Validation: `russet-hdiutil` compares its extraction of APFS images against
the same images mounted by macOS.
