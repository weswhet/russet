---
title: Native Apple formats
description: How Russet reads and writes disk images, packages, archives, and code signatures without Apple's tools, and where that differs from macOS.
---

On Linux, Russet runs macOS recipes without Apple's command-line tools. It
reads and writes Apple's file formats itself, with built-in replacements for
`hdiutil`, `pkgutil`, `pkgbuild`, `codesign`, and the other tools recipes
depend on. On macOS, Russet still calls Apple's tools.

## Replacements

| Apple tool | What Russet replaces | Notes |
| --- | --- | --- |
| `hdiutil` | `attach` (as extraction), `imageinfo`, and `create` | Reads `UDZO`, `UDBZ`, `ULFO`, `ULMO`, `UDCO`, and `UDRO` images with HFS+, HFSX, or APFS volumes. Creates HFS+ images only. |
| `ditto` | `-x -k` for zip archives and `-x` for cpio archives | Keeps file modes, symbolic links, hard links, and the extended attributes macOS stores in AppleDouble files. |
| `aa` | `extract` | Reads Apple Archives compressed with LZFSE, zlib, LZMA, or LZ4. |
| `xar` | Reading and writing flat package containers | Rejects archives whose table of contents fails its checksum or points outside the file. |
| `pkgutil` | `--expand`, `--flatten`, and `--check-signature` | |
| `pkgbuild` | `--analyze` and component package builds | Records the owners and modes Russet's macOS helper would, without running as root. |
| `mkbom`, `lsbom` | Writing and listing bills of materials | |
| `installer` | `-query RestartAction` | Doesn't install packages. |
| `codesign` | `--verify` with `--deep`, `--strict`, and a requirement | |
| ImageIO | Converting app icons to PNG for Munki | Chooses the same image as macOS. |

To compare a replacement with Apple's tool on macOS, set the `RUSSET_NATIVE`
environment variable. For details, see
[Environment variables](/reference/environment-variables/).

## Security model

Disk images, packages, and archives that recipes download aren't trusted.
Russet's readers check every size and offset before using it, fail on
anything they don't understand, and enforce limits on total size, entry
count, path depth, and name length. Extraction never follows a symbolic link
in the destination or the archive, and it never writes outside the
destination folder.

Signature checks fail closed: an unknown certificate extension, hash type,
or requirement clause is a failure, not a pass. The native verifier must
never accept a signature that Apple's tools reject. Rejecting one that
Apple's tools accept is a compatibility bug.

- Russet trusts only Apple's root certificate authorities.
- Russet doesn't check notarization, stapled tickets, certificate
  revocation, or Gatekeeper policy.
- Requirements that use the `notarized` clause fail on Linux.

## Differences from macOS

- **Disk images:** Russet extracts each image into a scratch folder instead
  of mounting it, so the folder needs free space about the size of the
  image's contents. Linux can't hard-link folders, so folder hard links
  become copies. Russet can't read encrypted, NDIF, or split images, or APFS
  snapshots.
- **Creating disk images:** Russet writes HFS+ volumes. When a recipe asks
  for APFS, DmgCreator's default, it writes HFS+ and logs a warning.
  Extended attributes aren't copied into the image.
- **Packages:** Russet can't sign packages. Packages, disk images, and icons
  that it builds aren't byte-identical to the ones Apple's tools build, so
  their hashes, such as Munki's `installer_item_hash`, differ.
- **Installer choices:** Munki's `installerChoices` option needs macOS,
  because `Distribution` files can choose packages with JavaScript.
- **Apple Archive:** archives compressed with LZBITMAP aren't supported,
  because the format isn't documented.
- **Paths outside disk images:** file systems on Linux are case-sensitive.
  When a path in a recipe matches nothing, Russet tries it again ignoring case, as
  it would resolve on macOS, so `CyberDuck.app` finds `Cyberduck.app`. Folder
  listings aren't sorted, so a glob that matches several items can pick a
  different one than on macOS.
- **Extended attributes:** file systems on Linux limit their size, and Linux
  doesn't allow them on symbolic links. Russet keeps the ones it can't store
  in a hidden `.russet-xattrs` folder at the top of the extraction, where its
  code signature checks find them. Copying a file elsewhere doesn't copy
  those attributes.
- **Log text:** messages from the replacements differ from the text
  `codesign`, `pkgutil`, and the other tools print.

## What's next

- [Platform support](/concepts/platform-support/)
- [Processors](/reference/processors/)
