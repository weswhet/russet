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
