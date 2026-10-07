# hdiutil

Russet's `russet-hdiutil` crate reads and creates disk images on Linux. These
notes record the behavior it matches, observed with `hdiutil`, `fsck_hfs`,
and `fsck_apfs` on macOS 27.0 (build 26A428). The tests in
`rust/crates/hdiutil` check on macOS CI that `hdiutil` reports the same
format for native images, mounts them with the same tree, and that
`fsck_hfs -n` finds no problems.

## imageinfo and attach

- The format comes from the block chunk types in the UDIF resource fork:
  zlib is `UDZO`, bzip2 `UDBZ`, LZFSE `ULFO`, LZMA `ULMO`, ADC `UDCO`, and an
  image of only raw and zero-fill chunks is `UDRO`.
- A license agreement is an `LPic` resource in the resource fork.
- On HFS+, file hard links point to entries named `iNode<number>` in the
  hidden `HFS+ Private Data` folder, whose name starts with four NUL
  characters. Folder hard links point into `.HFS+ Private Directory Data\r`.
  Linux can't recreate folder hard links, so the extraction copies them.

## create

- `hdiutil create -srcfolder` names the volume after the source folder.
- HFS+ stores names decomposed (NFD), except for the code point ranges TN1150
  leaves composed: U+2000–U+2FFF, U+F900–U+FAFF, and U+2F800–U+2FAFF.
  macOS doesn't find a precomposed name on a volume that stores it composed.
- macOS refuses to mount an HFS+ volume whose journal is only a stub, so the
  native writer always writes a non-journaled volume, even for
  `Journaled HFS+`. A read-only image doesn't need a journal.
- With `-layout NONE`, `hdiutil` writes one partition named
  `whole disk (Apple_HFS : 0)` and no partition map. The UDIF trailer then
  has image variant 2, and the partition's block map has block descriptor
  −2 (`0xFFFFFFFE`). The native writer uses this layout. On macOS 27,
  `hdiutil imageinfo` fails on such an image with variant 1, which means a
  device image with a partition map; variant 0 works there but `hdiutil`
  reports a corrupt image on macOS 15.
- The trailer's reserved area before the master checksum is 120 bytes, so
  the master checksum is at offset 0x160, the image variant at 0x1E8, and
  the sector count at 0x1EC.
- Every image has a 1032-byte `plst` resource, zero except bytes 517 and 519,
  which are 1.
- `fstool` 0.4.35's APFS writer produces a container that `fsck_apfs` rejects
  ("spaceman ip block count is bad") and macOS won't mount. The native
  `create` writes HFS+ when APFS is requested and reports the substitution.
