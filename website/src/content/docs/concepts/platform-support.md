---
title: Platform support
description: Which recipe operations work on macOS, Linux, and Windows, and which native tools they use.
---

Russet runs on macOS, Linux, and Windows, but some recipe operations depend on
tools that only one operating system provides. This page explains which
operations work on each platform.

## How platform limits work

Every platform registers the same processors, so a recipe loads on any
platform. When a processor needs a tool that the current platform doesn't
have, the processor fails with an error that names the limitation, such as
`Disk image operations are only supported on macOS and Linux`. Russet doesn't skip the
step or substitute different behavior.

Recipes that only download software and read metadata usually run on every
platform. Recipes that read disk images run on macOS and Linux. Recipes that
build new packages or verify macOS code signatures need macOS.

## Operations by platform

The following table shows how each kind of operation runs on each platform:

| Operation | macOS | Linux and Windows |
| --- | --- | --- |
| Downloads and web requests | curl | curl |
| Zip and tar archives | `ditto` and `tar` | On Linux, a built-in replacement for `ditto` that keeps file modes, symbolic links, and extended attributes. On both, a built-in extractor for tar archives that use gzip, bzip2, or xz compression. On Windows, zip archives lose file modes and symbolic links. |
| Disk images | `hdiutil` | Linux: a built-in reader for read-only `.dmg` images in the `UDZO`, `UDBZ`, `ULFO`, `ULMO`, `UDCO`, and `UDRO` formats with HFS+ or APFS volumes. Creating images, encrypted images, and ISO images aren't supported. Windows: not supported |
| Building and installing packages | Helper services, `pkgbuild`, and `installer` | Not supported |
| Expanding and flattening packages | `pkgutil`, `xar`, and `ditto` | Linux: built-in replacements that expand and flatten flat packages and unpack gzip and pbzx payloads. Windows: not supported |
| macOS code signatures | `codesign` and `pkgutil` | Not supported |
| Authenticode signatures | Not supported | Windows: `SignToolVerifier` with `signtool.exe` |
| Chocolatey packages | Not supported | Windows: `ChocolateyPackager` with `choco.exe` |
| `StopProcessingIf` predicates | Foundation predicates | A documented subset of predicate syntax |
| Munki metadata for packages and disk images | Supported | Not supported |
| Munki catalogs and pkginfo edits | Supported | Supported |
| Preferences | `com.github.autopkg` domain | `config.plist` or `config.json` file |

## Disk images on Linux

Linux can't mount Apple disk images, so Russet reads the image itself and
copies each volume's contents into a scratch folder, where processors see the
same files, permissions, symbolic links, hard links, and extended attributes
that macOS shows for the mounted image. Transparently compressed files are
decompressed. The copy is removed when the recipe finishes, and a recipe that
opens the same image in several steps reads it once.

The scratch folder needs free space about the size of the image's contents.
By default it's in the system temporary folder; set `RUSSET_SCRATCH_DIR` to
use another location. Because Linux can't hard-link folders, folder hard
links become copies.

## Predicates on Linux and Windows

`StopProcessingIf` evaluates a predicate to decide whether to stop a recipe.
On macOS, Russet uses the same Foundation predicate engine as Python AutoPkg.
On Linux and Windows, Russet evaluates a bounded subset of the predicate
syntax and rejects predicates outside that subset instead of guessing.

## Munki on Linux and Windows

Russet generates Munki metadata natively, but reading a package or a disk
image needs macOS tools. As a result, importing a new package or disk image
into Munki needs macOS. Processors that only edit metadata, such as
`MunkiPkginfoMerger`, and the `MakeCatalogsProcessor` catalog builder work on
every platform.

On Linux and Windows, the `_metadata` dictionary in a generated pkginfo
records the platform name instead of a macOS version.

## What's next

- [Processors](/reference/processors/)
- [System requirements](/get-started/requirements/)
- [Import software into Munki](/guides/import-into-munki/)
