# Russet changes

## 0.1.0 (unreleased)

The first Russet release. Release archives are named `russet-VERSION-TARGET`.
The installed command is `autopkg`, and `autopkg version` reports AutoPkg
compatibility version 3.0.0.

- Native Rust recipe engine, CLI, 46 built-in processors, macOS helpers, and
  Munki FileRepo support. Archives support macOS arm64 and Intel, Windows
  x86-64, and Linux x86-64, with installation and rollback support.
- Native ports of 12 primary-repository processors and one shared-recipe alias,
  for 58 implementations under 59 names.
- `run` and `install` find recipes by short name at the top level of each search
  folder and one folder below it, like Python AutoPkg. Identifier lookup covers
  the same two levels.
- Only `RECIPE_REPO_DIR` and the repositories in `RECIPE_REPOS` count as recipe
  repositories for trust records, so overrides verify when `autopkg` runs from
  any folder, including the home folder or `/`.
- The repository contains no Python. Packaging, installer tests, and release
  promotion run as `cargo xtask` commands, and the comparisons with Python
  AutoPkg run in [russet-compat](https://github.com/weswhet/russet-compat).
- On Linux, Unarchiver extracts zip archives and gzip-compressed cpio archives
  with a built-in replacement for `ditto`. It keeps file modes, symbolic links,
  and the extended attributes that macOS stores in AppleDouble members, so app
  bundles from zip archives stay intact. `USE_PYTHON_NATIVE_EXTRACTOR` now
  defaults to false on Linux. Setting `RUSSET_NATIVE=ditto` uses the same
  replacement on macOS.
- On Linux, processors read paths inside disk images, and AppDmgVersioner
  works, with a built-in reader for `.dmg` images in the `UDZO`, `UDBZ`,
  `ULFO`, `ULMO`, `UDCO`, and `UDRO` formats containing HFS+, HFSX, or APFS
  volumes. Each image is extracted once per recipe into `RUSSET_SCRATCH_DIR`
  or the system temporary folder. `RUSSET_NATIVE=hdiutil` uses the same
  reader on macOS.
- On Linux, FlatPkgUnpacker, FlatPkgPacker, PkgPayloadUnpacker, and
  PkgExtractor use built-in replacements for `pkgutil --expand`,
  `pkgutil --flatten`, `xar`, `ditto`, and `aa`. They read gzip and pbzx
  payloads. The xar reader rejects packages whose table of contents fails its
  checksum, has more than one checksum element, or points outside the file.
- On Linux, MunkiImporter and MunkiInfoCreator read packages and disk images.
  `RestartAction` comes from a built-in replacement for
  `installer -query RestartAction`, and the image format from one for
  `hdiutil imageinfo`. `installerChoices` still needs macOS.
- On Linux, MunkiImporter's `extract_icon` reads app icons from packages and
  disk images and converts them to PNG, choosing the same image as macOS.
  `RUSSET_NATIVE=icons` uses the same code on macOS.
- On Linux, PkgPayloadUnpacker and `extract_icon` read Apple Archive payloads
  with a built-in replacement for `aa extract`. It reads archives compressed
  with LZFSE, zlib, LZMA, or LZ4, or stored uncompressed, and rejects
  LZBITMAP, which isn't documented. `RUSSET_NATIVE=aa` uses it on macOS.
- On Linux, CodeSignatureVerifier checks installer package signatures with a
  built-in replacement for `pkgutil --check-signature`. It verifies the RSA
  and CMS signatures over the package's table-of-contents checksum, the
  trusted timestamp, and the certificate chain to Apple's root certificates,
  which are the only roots it trusts. It doesn't check notarization or
  revocation.
- On Linux, CodeSignatureVerifier checks app signatures with a built-in
  replacement for `codesign --verify --deep --strict -R`: every architecture's
  code directories, the CMS signature and timestamp, the chain to Apple's
  roots, the resource seal, nested code, and the requirement. It supports the
  requirement clauses the core recipes use and rejects others, such as
  `notarized`, rather than ignoring them.
- On Linux, PkgCreator and AppPkgCreator build packages in-process, with
  built-in replacements for `pkgbuild` and `mkbom`. The packages record the
  same owners and modes as the macOS helper without running as root;
  `RUSSET_PKG_OWNER` sets the owner for files the running user owns.
- On Linux, DmgCreator creates disk images with a built-in replacement for
  `hdiutil create`, in the `UDZO`, `UDBZ`, `ULFO`, and `UDRO` formats. It
  writes HFS+ volumes; when a recipe asks for APFS, the default, it writes
  HFS+ and logs a warning, because there's no APFS writer that macOS accepts.
  Extended attributes aren't copied into the image.
