# Russet changes

## 0.1.0 (unreleased)

The first Russet release. Release archives are named `russet-VERSION-TARGET`.
The installed command is `russet`, and `russet version` reports AutoPkg
compatibility version 3.0.0.

- Native Rust recipe engine, CLI, 46 built-in processors, macOS helpers, and
  Munki FileRepo support. Archives support macOS arm64 and Intel, Windows
  x86-64, and Linux x86-64, with installation and rollback support.
- Native ports of 12 primary-repository processors and one shared-recipe alias,
  for 58 implementations under 59 names.
- `run` and `install` find recipes by short name at the top level of each search
  folder and one folder below it, like Python AutoPkg. Identifier lookup covers
  the same two levels.
- `run` and `install` accept `-j/--jobs N`, and read the `RussetJobs`
  preference, to run up to `N` recipes at the same time; `0` means one per
  CPU. The default stays one at a time. Each output line starts with the
  recipe's name, each recipe prints a line with its run time when it finishes
  or fails, and reports keep list order. Recipes that share a cache
  folder, a disk image, packaging and installs, and `MakeCatalogsProcessor`
  recipes still run one at a time, and Munki catalogs are written atomically.
- In a terminal on macOS or Linux, `run` and `install` show a live display:
  a block for each recipe with a spinner and run time, a line for each
  processor with its most recent message, and counts of running, waiting, and
  finished recipes. Finished recipes stay in the terminal. `--plain`, `-v`, a
  `dumb` terminal, and piped output keep AutoPkg's plain output.
- Only `RECIPE_REPO_DIR` and the repositories in `RECIPE_REPOS` count as recipe
  repositories for trust records, so overrides verify when `russet` runs from
  any folder, including the home folder or `/`.
- The command is `russet`, and it accepts the same verbs and options as
  `autopkg`. On macOS, launchd runs the helper services as `russet --server`
  and `russet --installd`; there are no separate helper executables.
- Russet installs in `/opt/russet`, links `/usr/local/bin/russet`, and on macOS
  uses its own launchd jobs (`com.github.weswhet.russet.server` and
  `com.github.weswhet.russet.installd`), sockets, and logs. It doesn't change
  a Python AutoPkg installation.
- Russet has its own preferences: the `com.github.russet` domain on macOS, and
  `config.plist` or `config.json` in `~/.config/Russet` on Linux or
  `%LOCALAPPDATA%\Russet` on Windows. Russet doesn't read or change AutoPkg's
  preferences; `repo-add` and `repo-delete` save changes to Russet's
  preferences or the `--prefs` file. On Linux and Windows, these commands no
  longer need `--prefs` when no preference file exists. With `--prefs`, Russet
  saves only that file's values and the changed keys.
- New `russet migrate` copies AutoPkg's preferences into Russet's preferences.
  `--dry-run` shows the changes without saving them, and `--force` replaces
  values that differ. It copies every key and value type, including keys in
  `/Library/Preferences` and from configuration profiles, and AutoPkg's
  default search and override folders when they're unset. It doesn't move any
  folders, so paths that AutoPkg's preferences name keep working.
- On macOS, Russet reads keys in `/Library/Preferences/com.github.russet.plist`
  and keys that configuration profiles set, as Python AutoPkg does for its
  domain.
- On macOS, decimal numbers from the preference domain become whole numbers at
  the top level and inside arrays, as in Python AutoPkg, so recipes get the
  same values.
- `info` wraps long values like Python's `pprint`, and shows dates, data, and
  nested arrays from the macOS preference domain the way Python AutoPkg does.
- Trust records include processors that Russet doesn't have, such as those in
  recipe repositories, with the same hashes as Python AutoPkg, so
  `verify-trust-info`, `update-trust-info`, and `make-override` work for
  recipes that use them. Russet still can't run those processors. `repo-delete` keeps the folder of a recipe
  repository that AutoPkg's preferences list.
- Clones, overrides, recipes, the cache, and the recipe map default to folders
  in `~/Library/Russet` and `/Library/Russet`.
- The GitHub token file defaults to `~/.russet_gh_token`, and Russet reads
  `~/.autopkg_gh_token` when only that file exists. `RUSSET_RECIPE_MAP_PATH`
  and `RUSSET_DISABLE_RECIPE_MAP` replace the `AUTOPKG_` names, which still
  work. The development variables are now `RUSSET_CACHE_DIR`, `RUSSET_DEBUG`,
  `RUSSET_PREFERENCES_FILE`, and `RUSSET_AUTOPKG_PREFERENCES_FILE`.
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
- On Linux, when a recipe's path matches nothing, Russet tries it again
  ignoring case, the way it resolves on case-insensitive macOS volumes. So
  does PkgPayloadUnpacker's `pkg_payload_path`, so `TempPackage.pkg/payload`
  finds `Payload`.
  Extended attributes that Linux file systems refuse, such as large resource
  forks and the signatures of non-Mach-O files, are kept in a
  `.russet-xattrs` folder at the top of the extraction for code signature
  checks.
- On Linux, PkgPayloadUnpacker and `extract_icon` read Apple Archive payloads
  with a built-in replacement for `aa extract`. It reads archives compressed
  with LZFSE, zlib, LZMA, or LZ4, or stored uncompressed, and rejects
  LZBITMAP, which isn't documented. `RUSSET_NATIVE=aa` uses it on macOS.
- On Linux, CodeSignatureVerifier checks installer package signatures with a
  built-in replacement for `pkgutil --check-signature`. It verifies the RSA
  and CMS signatures over the package's table-of-contents checksum, the
  trusted timestamp, and the certificate chain to Apple's root certificates,
  which are the only roots it trusts. Like `pkgutil`, it accepts RSA
  signatures over the checksum's digest and CMS signatures without signed
  attributes, which some signers write. It doesn't check notarization or
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
