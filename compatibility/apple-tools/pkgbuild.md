# pkgbuild

Russet's `russet-pkgbuild` crate builds component packages on Linux in place
of `pkgbuild`, and `autopkg-helpers` uses it in place of the `autopkgserver`
helper. These notes record the behavior it matches, observed with `pkgbuild`
on macOS 27.0 (build 26A428). The tests in `rust/crates/pkgbuild` compare the
two on every macOS CI runner, and Apple's `pkgutil`, `lsbom`, and `installer`
read the native packages there.

## Package layout

The xar archive holds `Bom` (zlib), `Payload` (stored), `Scripts` (stored,
when there are scripts), and `PackageInfo` (zlib), in that order. `Payload`
and `Scripts` are gzip-compressed odc cpio archives whose members are `.` and
`./path`.

## Payload contents

`pkgbuild` leaves out `.DS_Store`, `.svn`, `CVS`, and `.git` at any level, and
paths matching `--filter`. Other names, including real files named `._name`,
are kept. On hosts that add extended attributes such as
`com.apple.provenance`, `pkgbuild` also records AppleDouble `._name` entries;
those come from the host, not the root, and comparisons ignore them.

## PackageInfo

Without `--info`, the root element has `overwrite-permissions="true"`,
`relocatable="false"`, and `postinstall-action="none"`. With `--info`, the
template's attributes and child elements are kept, except that `identifier`
and `version` come from the command line and `auth` is always `root`.
`--install-location` and `--min-os-version` set `install-location` and
`minimumSystemVersion`.

After the template's children come `payload` (`numberOfFiles` is the number
of BOM entries), a `bundle` element for each top-level and nested bundle,
then `bundle-version`, `upgrade-bundle`, `update-bundle`,
`atomic-update-bundle`, `strict-identifier`, and `relocate` lists, and with
`--scripts` a `scripts` element naming `preinstall` and `postinstall` with a
600-second timeout.

`installKBytes` tracks the uncompressed payload's size in KiB. Russet rounds
the unpadded cpio size up; `pkgbuild` measured the padded archive in a way
that couldn't be confirmed on a host that adds AppleDouble entries, so the
value can differ by 1.

## --analyze

Folders with a bundle extension (such as `.app`, `.bundle`, or `.framework`)
and an Info.plist with `CFBundleIdentifier` are bundles. Top-level bundles
get `BundleOverwriteAction` `upgrade` and `BundleIsVersionChecked`; bundles
inside them become `ChildBundles` with an empty overwrite action. Apps get
`BundleHasStrictIdentifier`. Russet's helper always sets
`BundleIsRelocatable` to false.

## autopkgserver ownership rules

The helper copies the root into a `root:admin` (0:80) folder with mode
`01775`, so files keep their owner, then applies `chown` entries in order: a
file gets the owner and mode; a folder gets the owner on itself and
everything in it, and the mode only on what's in it. It then runs
`pkgbuild --ownership preserve`. The native builder records the same owners
and modes in the payload and BOM without changing the disk. Files the
invoking user owns are recorded as `RUSSET_PKG_OWNER` (default `501:20`),
and owner names resolve through macOS's standard accounts.
