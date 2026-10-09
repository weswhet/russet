# Russet patch

This directory vendors hfsplus 0.4.0 from crates.io
(<https://github.com/Dil4rd/dpp>). Its MIT license is retained in `LICENSE`.

Russet's `russet-hdiutil` crate uses hfsplus to read HFS+ and HFSX volumes on
Linux. Each change is marked `Russet patch` in the source:

- **Fork type for overflow extents.** `extents::read_fork_data` always looked
  up overflow extents for the data fork, so a resource fork with more than
  eight extents, including the payload of a compressed file, was read wrongly.
  It now takes the fork type, and every caller passes the right one.
- **Finder info.** `CatalogFile` and `CatalogFolder` keep the 32 bytes of
  Finder info that the parser skipped. They identify file and directory hard
  links (`hlnk`/`hfs+` and `fdrp`/`MACS`), and macOS presents them as the
  `com.apple.FinderInfo` extended attribute.
- **Record-based access.** `catalog::list_records` and the `HfsVolume`
  methods `children`, `read_record_to`, `read_record_resource_fork`,
  `is_compressed`, `list_xattrs_by_id`, and `get_xattr_by_id` work from
  catalog IDs and records, so callers never look a name up. Name lookups use
  a partial case-folding table that doesn't match HFS+'s FastUnicodeCompare
  (for example, it doesn't sort NUL last), so lookups of some names, such as
  the private hard-link folder, fail.
- **`record` and `read_resource_fork`.** Path-based access to a catalog
  record and a resource fork.

Validation: `russet-hdiutil` compares its extraction of HFS+ and HFSX images,
with hard links, resource forks, Finder info, extended attributes, and
transparently compressed files, against the same images mounted by macOS.
- **Record flags.** `CatalogFile::flags` exposes the catalog file record's
  flags, so callers can tell hard links (`kHFSHasLinkChainMask`, 0x20) from
  Finder aliases, which share their `fdrp`/`MACS` and `hlnk`/`hfs+` types.
- **Fragmented B-trees.** A catalog or attributes file with more than eight
  extents keeps the rest in the extents overflow file, and B-tree reads only
  followed the inline eight ("fork offset exceeds extent capacity").
  `HfsVolume` now looks those extents up when it opens the tree, and
  `BTreeHeaderRecord::overflow_extents` holds them.
