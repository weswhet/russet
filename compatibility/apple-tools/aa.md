# aa (Apple Archive)

Russet's `russet-aa` crate reads Apple Archives on Linux, where package
payloads and Munki icon lookups would otherwise need `aa extract`. These
notes record the format details it relies on, observed with `aa` on macOS 27.0
(build 26A428). The tests in `rust/crates/aa` check, on every macOS CI runner,
that native extraction produces the same tree as `aa extract` for archives
`aa archive` writes with each codec.

## Stream

- `aa archive` compresses by default. The stream starts with `pbz` and a
  codec letter: `e` (LZFSE, the default), `z` (zlib), `x` (LZMA, as xz), `4`
  (LZ4), or `b` (LZBITMAP). A big-endian 64-bit block size follows (4 MiB by
  default; `-b` changes it).
- Each chunk is a big-endian 64-bit decoded size, a 64-bit stored size, and
  the stored bytes. A chunk whose two sizes match is stored uncompressed.
- LZ4 chunks use Apple's block framing: `bv41` (decoded size, stored size,
  raw LZ4 block), `bv4-` (size, uncompressed bytes), and `bv4$` at the end,
  with little-endian 32-bit sizes.
- LZBITMAP isn't documented, so Russet rejects those streams.
- `-a raw` writes the entries with no `pbz` wrapper.
- An LZMA archive has the same `pbzx` wrapper as package payloads that
  `pkgbuild` writes, which hold cpio archives instead. Russet checks the
  decoded bytes for an entry header to tell them apart.

## Entries

- Each entry starts with `AA01` (or the older `YAA1`) and a little-endian
  16-bit size of the whole header.
- Fields are a 3-letter key and a subtype letter: `*` (no value); `1`, `2`,
  `4`, `8` (little-endian unsigned integers of that size); `P` (16-bit length
  and bytes); `T` (64-bit seconds and 32-bit nanoseconds); `S` (64-bit
  seconds); `F`–`J` (CRC-32, SHA-1, SHA-256, SHA-384, and SHA-512 digests);
  and `A`, `B`, `C` (blobs with 16-, 32-, or 64-bit sizes).
- Blob bytes follow the header, in the order of their fields.
- `aa archive` writes `TYP`, `PAT`, `LNK` (symlink target), `UID`, `GID`,
  `MOD`, `FLG`, `CTM`, `MTM`, `DAT` (file data), and `XAT` (extended
  attributes) by default. The root folder has an empty `PAT`.
- `TYP` is `D` (folder), `F` (file), or `L` (symlink), among others.
- Hard links are separate `F` entries that each carry the data and share an
  `HLC` (hard link cluster) value. `aa extract` links them.
- An `XAT` blob holds entries of a little-endian 32-bit size (counting
  itself), a NUL-terminated name, and the value.
