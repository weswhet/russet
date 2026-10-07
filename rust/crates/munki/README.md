# Native Munki compatibility

The native implementation targets **Munki 7.2.0.5787**, commit
`8896fe831e870732aac760f76566762fc35d5d00`. It uses the frozen option catalog in
`compatibility/munki-makepkginfo-options.json`. Runtime execution never invokes
`makepkginfo` or loads Munki Python libraries.

Implemented workflows include flat and bundle package and disk-image metadata, installs
items, metadata-only options, package scripts, installer choices, FileRepo catalog
indexes and duplicate matching, package/pkginfo copying, receipt editing, and
application icon extraction. Icon conversion uses native ImageIO with the pinned
reference's representation selection. Package and image inspection uses native
`xar`, `installer`, `pkgutil`, and `hdiutil` tools on macOS.

Mounted image roots resolve to their backing image. Existing image mounts are
borrowed and left mounted after inspection. Multi-volume images use the first
volume, matching the pinned reference, and owned attachments detach by whole-disk
device. Private mount directories distinguish newly created mounts from reused
attachments. Cleanup never recursively deletes a mount directory. Staged macOS installer metadata reads
legacy `InstallInfo.plist` or a nested `SharedSupport.dmg`; it never executes
`startosinstall` or installs an operating system. Bundle payload icons are streamed
from their archive into temporary files without installing the payload. Flat
packages use BOM-listed applications and selective payload reads. Both traditional
archives and Apple Archive payloads are supported; package icon selection includes
only `.icns` members, matching the pinned reference.

Metadata option aliases come from the pinned catalog. Unknown options, invalid
architectures, malformed dates, duplicate installer-environment keys, and invalid
version options return errors. Metadata overrides preserve plist dates and data.
`force_install_after_date` passed through the makepkginfo option remains a string,
matching the Swift reference; the AutoPkg `pkginfo` override converts a matching
UTC date string into a plist date.

Catalog rebuilding remains outside the importer. Duplicate checks use
`catalogs/all`; tests explicitly update that fixture between imports. Package
and pkginfo filenames preserve the reference's version suffix and collision
counter behavior. Custom repository backends and Python library loading reject
before executing processor work.

## Remaining compatibility gaps

- Legacy bundle packages still depend on the native `installer` restart query.
  Some current macOS versions reject them, as does the pinned Munki reference.
  Metadata receipt parsing is implemented, and applicable native-tool failures
  are preserved. Importing a bundle directory directly still follows AutoPkg's
  file-copy restriction; place it in a disk image for repository import.
- `--version` and `-V` reject as output-mode requests rather than metadata
  options. `--help` is not accepted inside processor metadata generation.
- NonmacOS metadata-only `_metadata.os_version` currently identifies the platform
  rather than its full operating-system version.

These gaps are known limits of the current implementation.

## Verification

Run portable checks with `cargo test -p autopkg-munki`. Development reference
tests are ignored by default because they require the exact installed Munki
release or macOS tools. Run them explicitly with
`cargo test -p autopkg-munki -- --include-ignored` on the reference host.

Native fixtures cover flat and legacy bundle package behavior, drag-and-drop
and mounted disk-image inputs, legacy and modern staged macOS installer
metadata, metadata-only options, installs items, and FileRepo indexes. The
FileRepo index and matching test compares with output frozen from the pinned
Python implementation in `tests/fixtures/`. Comparisons of complete imports
with Python AutoPkg run in [russet-compat](https://github.com/weswhet/russet-compat);
they compare typed plist values and package contents, and normalize only
repository fixture roots and `_metadata.creation_date`.

On the current reference host, the synthetic legacy bundle fixture exercises a
matching rejection from the native installer tool. Separate tests verify bundle
receipt parsing and icon extraction. Staged-installer fixtures contain synthetic
metadata and a nonexecuted placeholder, not a real macOS installer.

Native platform fixtures verify a synthetic two-volume GPT image detaches every
volume, a second attachment attempt preserves the first mount, and mount-directory
cleanup leaves nonempty contents untouched. Icon fixtures cover multiple applications
in a flat package and traditional versus Apple Archive bundle payloads.

Implementation references:

- [Munki metadata generation](https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/shared/admin/pkginfolib.swift)
- [Package inspection](https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/shared/utils/pkgutils.swift)
- [Icon conversion](https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/shared/utils/iconutils.swift)
- AutoPkg [`MunkiImporter.py`](https://github.com/autopkg/autopkg/blob/c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c/Code/autopkglib/MunkiImporter.py) and
  [`munkirepolibs/AutoPkgLib.py`](https://github.com/autopkg/autopkg/blob/c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c/Code/autopkglib/munkirepolibs/AutoPkgLib.py) at the
  pinned reference commit.
