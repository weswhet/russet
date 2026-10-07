# Russet patch

This directory vendors udif 0.4.2 from crates.io
(<https://github.com/Dil4rd/dpp>). Its MIT license is retained in `LICENSE`.

Russet's `russet-hdiutil` crate uses udif to read UDIF disk images on Linux.
One change was made:

- **ADC blocks.** Legacy `UDCO` images compress data with Apple Data
  Compression, which udif rejected as unsupported. `src/reader.rs` now decodes
  ADC blocks with the `adc` crate in every decompression path, and
  `Cargo.toml` adds that dependency. Each changed site is marked
  `Russet patch`.

Validation: `russet-hdiutil` extracts a `UDCO` image converted by
`hdiutil convert` and compares the result with the same image mounted by
macOS, both at test time on macOS and through the committed fixture on every
Unix platform.
