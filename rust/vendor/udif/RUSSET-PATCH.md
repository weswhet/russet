# Russet patch

This directory vendors udif 0.4.2 from crates.io
(<https://github.com/Dil4rd/dpp>). Its MIT license is retained in `LICENSE`.

Russet's `russet-hdiutil` crate uses udif to read and write UDIF disk images
on Linux. These changes were made:

- **ADC blocks.** Legacy `UDCO` images compress data with Apple Data
  Compression, which udif rejected as unsupported. `src/reader.rs` now decodes
  ADC blocks with the `adc` crate in every decompression path, and
  `Cargo.toml` adds that dependency. Each changed site is marked
  `Russet patch`.
- **Trailer layout.** `src/format.rs` read and wrote the reserved area before
  the master checksum as 64 bytes instead of 120, so the master checksum,
  image variant, and sector count landed at the wrong offsets. macOS 15's
  `hdiutil` rejected written images as corrupt.
- **Single-volume images.** `src/writer.rs` writes image variant 2 and block
  descriptor `0xFFFFFFFE` for an image with one partition and no partition
  map, as `hdiutil create -layout NONE` does. It also records the buffers
  needed (the chunk's sectors plus 8) and adds the `plst` resource `hdiutil`
  always writes.
- **Streaming partitions.** `DmgWriter::add_partition_from_reader` reads a
  partition one chunk at a time and checksums it as it goes;
  `add_partition` calls it. Before, `add_partition` needed the whole
  partition in memory and copied it again to checksum it.

Validation: `russet-hdiutil` extracts a `UDCO` image converted by
`hdiutil convert` and compares the result with the same image mounted by
macOS, both at test time on macOS and through the committed fixture on every
Unix platform.
- **Untrusted sizes.** `src/reader.rs` checks the trailer's plist and data
  fork ranges against the file, streams the data-fork checksum, validates
  every block run when an image opens (stored data inside the data fork,
  at most 64 MiB per run held in memory), and writes zero-fill runs in
  pieces. Before, a crafted trailer or block map could make it allocate
  any amount of memory (found by the `hdiutil_extract` fuzz target). Each
  partition's size must also fit in 64 bits, every run must lie inside its
  partition, and the padding after the last run is written in pieces; a
  1,311-byte image declaring 2^54 sectors used to panic. LZFSE blocks are decoded
  as a stream capped one byte past their declared size, instead of in full.
