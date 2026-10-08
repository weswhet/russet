---
title: Release notes
description: What each Russet release contains, starting with Russet 0.1.0.
---

This page lists what each Russet release contains. Russet distribution
versions are separate from the AutoPkg compatibility version that
`russet version` reports, which is 3.0.0.

## Russet 0.1.0

Russet 0.1.0 is the first Russet release. It isn't published yet. Until
then, [build Russet from source](/get-started/install/).

### Recipes and commands

- A native recipe engine and the `russet` command, with the commands,
  options, preference keys, and file locations of Python AutoPkg 3.0.0.
- Plist and YAML recipes, parent recipes, overrides, and trust information.
- Recipe lookup by short name at the top level of each folder and one level
  down, as in Python AutoPkg, so you can run a recipe from a recipe
  repository by name, such as `russet run Firefox.download`.
- Trust verification that rejects trust information in overrides inside
  recipe repositories. For details, see
  [Recipe trust](/concepts/recipe-trust/#where-trusted-overrides-must-be).

### Processors

- The 46 AutoPkg built-in processors.
- Native versions of 12 processors from the `autopkg/recipes` repository, so
  recipes that use them don't fail as custom processors:
  - `AdobeAcrobatProUpdateInfoProvider`
  - `AdobeFlashURLProvider`
  - `AdobeReaderRepackager`
  - `AdobeReaderURLProvider`
  - `AutoPkgSourceFinder`
  - `BarebonesURLProvider`
  - `GenerateRelocatablePython`
  - `MakeCatalogsProcessor`
  - `MozillaURLProvider`
  - `MSOfficeMacURLandUpdateInfoProvider`
  - `PuppetlabsProductsURLProvider`
  - `SassafrasK2ClientCustomizer`
- The shared processor reference
  `com.github.autopkg.AutoPkgGitMaster/GenerateRelocatablePython`.
- Munki pkginfo generation and catalog rebuilding for file-based Munki
  repositories.

### Platforms and installation

- Archives for macOS on Apple silicon and Intel, Linux x86-64, and Windows
  x86-64. Release archive names use the form `russet-VERSION-TARGET`, such as
  `russet-0.1.0-aarch64-apple-darwin.tar.gz`.
- Native macOS helper services for building and installing packages, run by
  the `russet` command.
- Installers that put Russet in `/opt/russet`, separate from Python AutoPkg,
  and keep the previous Russet installation so that you can roll back.
