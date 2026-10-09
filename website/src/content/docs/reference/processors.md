---
title: Processors
description: Every processor that Russet implements, the platforms where each one works, and how Russet handles processors that it doesn't implement.
---

A processor is a step in a recipe's `Process` array, such as `URLDownloader`
or `MunkiImporter`. Russet implements processors natively, with the same
names, input variables, output variables, and default values as AutoPkg.
This page lists every processor that Russet implements and the platforms
where each one works.

To print the list that your installation implements, run
`russet list-processors`. To see a processor's variables, run
`russet processor-info PROCESSOR`, replacing `PROCESSOR` with a name from
the following tables.

The **Platform** column uses the following values:

- **All:** the processor works on macOS, Linux, and Windows.
- **macOS** or **Windows:** the processor needs tools from that operating
  system. On other platforms, it fails with an error that names the
  limitation.

For more about platform differences, see
[Platform support](/concepts/platform-support/).

## AutoPkg built-in processors

Russet implements the 46 processors that AutoPkg 3.0.0 includes:

| Processor | Purpose | Platform | Notes |
| --- | --- | --- | --- |
| `AppDmgVersioner` | Reads the bundle identifier and version of the app in a disk image. | macOS, Linux | Mounts the disk image with `hdiutil` on macOS and reads it with a built-in reader on Linux. |
| `AppPkgCreator` | Builds a package from an app. | macOS, Linux | Uses the `russet-server` helper service on macOS and a built-in package builder on Linux. |
| `ChocolateyPackager` | Builds a NuGet package with `choco.exe`. | Windows | Rejects `license` and `contentFiles` values that are dictionaries or arrays. |
| `CodeSignatureVerifier` | Verifies the code signature of an app or installer package. | macOS, Linux | Uses `codesign` and `pkgutil` on macOS and built-in replacements on Linux, which trust only Apple's root certificates and reject requirement clauses they don't support, such as `notarized`. The `DISABLE_CODE_SIGNATURE_VERIFICATION` variable skips it on every platform. |
| `Copier` | Copies a file or folder. | All | Paths inside a disk image need macOS or Linux. |
| `DeprecationWarning` | Prints a deprecation warning for a recipe. | All | None. |
| `DmgCreator` | Creates a disk image from a folder. | macOS, Linux | Uses `hdiutil` on macOS. On Linux, a built-in replacement writes `UDZO`, `UDBZ`, `ULFO`, or `UDRO` images with an HFS+ volume, including when `dmg_filesystem` is APFS, and doesn't copy extended attributes. |
| `DmgMounter` | Base class for processors that mount disk images. | None | Fails if a recipe runs it directly. |
| `EndOfCheckPhase` | Marks where `russet run --check` stops. | All | Does nothing when it runs. |
| `FileCreator` | Creates a file with the content that you supply. | All | On Windows, `file_mode` only sets or clears the read-only attribute. |
| `FileFinder` | Finds a file that matches a pattern. | All | Supports only `find_method` `glob`. Paths inside a disk image need macOS or Linux. |
| `FileMover` | Moves or renames a file. | All | None. |
| `FindAndReplace` | Replaces text in a string variable. | All | None. |
| `FlatPkgPacker` | Flattens an expanded package. | macOS, Linux | Uses `pkgutil` on macOS and a built-in replacement on Linux. |
| `FlatPkgUnpacker` | Expands a flat package. | macOS, Linux | Uses `pkgutil`, or `xar` with `skip_payload`, on macOS and built-in replacements on Linux. A pattern inside a disk image must match exactly one file. |
| `GitHubReleasesInfoProvider` | Gets the most recent release of a GitHub project. | All | Uses curl and your GitHub token, if you set one. |
| `InstallFromDMG` | Copies items from a disk image to the startup volume. | macOS | Uses the `russet-installd` helper service. |
| `Installer` | Installs a package. | macOS | Uses the `russet-installd` helper service. The package must be in the recipe's cache or on a mounted disk image. |
| `MunkiCatalogBuilder` | Deprecated. | All | Prints a warning and does nothing. Use `MakeCatalogsProcessor` to rebuild catalogs. |
| `MunkiImporter` | Imports a package or disk image into a Munki repository. | macOS, Linux | Supports only `FileRepo` Munki repositories. Generates metadata natively and doesn't run `makepkginfo`. On Linux, the `installerChoices` option isn't supported. |
| `MunkiInfoCreator` | Creates a pkginfo file for a package or disk image. | macOS, Linux | Reads packages and disk images with macOS tools on macOS and built-in readers on Linux. |
| `MunkiInstallsItemsCreator` | Creates an `installs` array for a pkginfo file. | All | Some operations need macOS. |
| `MunkiOptionalReceiptEditor` | Edits the receipts in a pkginfo file. | All | None. |
| `MunkiPkginfoMerger` | Merges two pkginfo dictionaries. | All | None. |
| `MunkiSetDefaultCatalog` | Sets a pkginfo's catalog to the `munkiimport` default catalog. | All | Reads the `munkiimport` preference only on macOS. |
| `PackageRequired` | Fails the recipe if the `PKG` variable isn't set. | All | None. |
| `PathDeleter` | Deletes files and folders. | All | None. |
| `PkgCopier` | Copies a package. | All | Paths inside a disk image need macOS or Linux. |
| `PkgCreator` | Builds a package from a package root. | macOS, Linux | Uses the `russet-server` helper service on macOS. On Linux, a built-in package builder records the same owners and modes without running as root; it supports the `pkgbuild_args` `--install-location`, `--min-os-version`, `--compression legacy`, and `--filter`. |
| `PkgExtractor` | Extracts the contents of a bundle-style package. | macOS, Linux | Uses `ditto` on macOS and a built-in replacement on Linux. |
| `PkgInfoCreator` | Creates a `PackageInfo` file for a package. | All | None. |
| `PkgPayloadUnpacker` | Unpacks a package payload. | macOS, Linux | Uses `ditto`, with `aa` as a fallback, on macOS. On Linux, a built-in replacement reads gzip, pbzx, and Apple Archive payloads, except Apple Archives compressed with LZBITMAP. |
| `PkgRootCreator` | Creates a package root and its folder structure. | All | On Windows, only the read-only attribute of folder modes applies. |
| `PlistEditor` | Merges data into a property list file. | All | None. |
| `PlistReader` | Reads keys from a property list into variables. | All | Paths inside a disk image need macOS or Linux. |
| `SignToolVerifier` | Verifies an Authenticode signature. | Windows | Needs `signtool.exe` from the Windows SDK. |
| `SparkleUpdateInfoProvider` | Gets the download URL and version from a Sparkle feed. | All | Uses curl. |
| `StopProcessingIf` | Stops a recipe when a predicate is true. | All | On Linux and Windows, supports a subset of the predicate syntax. |
| `Symlinker` | Creates a symbolic link. | All | Behavior on Windows depends on your permission to create symbolic links. |
| `URLDownloader` | Downloads a file. | All | Caches downloads and skips unchanged files. Uses Russet's downloader, or curl when the [`UseRussetDownloader`](/reference/preferences/#downloads) preference or the recipe's `curl_opts` require it. |
| `URLDownloaderPython` | Downloads a file. | All | Kept for recipes that use this name. Russet uses its downloader or curl instead of Python, and ignores `curl_opts`. |
| `URLGetter` | Base class for processors that use curl. | None | Fails if a recipe runs it directly. |
| `URLTextSearcher` | Downloads text and matches a regular expression against it. | All | Uses curl. Follows Python regular expression syntax. |
| `Unarchiver` | Extracts zip and tar archives. | All | On macOS, uses `ditto` and `tar`. On Linux, uses a built-in replacement for `ditto` that keeps file modes, symbolic links, and extended attributes. On Windows, uses a built-in extractor that doesn't support `archive_format` `gzip`. |
| `VariableSetter` | Sets variables for later steps. | All | None. |
| `Versioner` | Reads a version from a property list. | All | Reads paths inside zip archives. Paths inside a disk image need macOS or Linux. |

## Processors from the AutoPkg recipes repository

The `autopkg/recipes` recipe repository supplies some processors as Python
files. Russet implements the following 12 of them natively. When a recipe
uses one of these names, Russet runs its own implementation and never runs the
recipe repository's Python file.

| Processor | Purpose | Platform | Notes |
| --- | --- | --- | --- |
| `AdobeAcrobatProUpdateInfoProvider` | Gets the most recent Adobe Acrobat Pro update. | All | `major_version` must be `9`, `10`, or `11`. |
| `AdobeFlashURLProvider` | Gets the most recent Adobe Flash Player download. | All | The vendor might have retired the download. |
| `AdobeReaderRepackager` | Repackages the Adobe Reader installer for deployment. | macOS | Uses `hdiutil` and `pkgutil`. |
| `AdobeReaderURLProvider` | Gets the most recent Adobe Acrobat Reader download. | All | None. |
| `AutoPkgSourceFinder` | Finds the source folder in an expanded AutoPkg archive. | All | None. |
| `BarebonesURLProvider` | Gets the most recent download of a Bare Bones Software product. | All | `product_name` must be `bbedit` or `yojimbo`. |
| `GenerateRelocatablePython` | Builds a relocatable Python framework. | macOS | Accepts only the pinned builder revision `8ee72fe`. Needs Git, curl, and network access. |
| `MakeCatalogsProcessor` | Rebuilds the catalogs of a Munki repository. | All | Supports a local path or a `file:` URL. Runs only when a recipe changed the Munki repository, or when you set `force_rebuild`. |
| `MozillaURLProvider` | Gets the most recent Firefox or Thunderbird download. | All | None. |
| `MSOfficeMacURLandUpdateInfoProvider` | Gets downloads and update information for Microsoft products for Mac. | All | Office 2016 products print a warning and stop. Microsoft Edge accepts only `latest` and named channels. |
| `PuppetlabsProductsURLProvider` | Gets the download URL of a Puppet product. | All | None. |
| `SassafrasK2ClientCustomizer` | Runs the vendor's `k2clientconfig` script on a K2Client installer. | All | The script that it runs might need a specific platform. |

## Shared recipe processor alias

Recipes can refer to a processor that another recipe supplies, with the form
`RECIPE_IDENTIFIER/PROCESSOR`. Russet recognizes one such name:

| Name | Runs |
| --- | --- |
| `com.github.autopkg.AutoPkgGitMaster/GenerateRelocatablePython` | Russet's `GenerateRelocatablePython` processor. |

Russet uses the referenced recipe's folder only to find the Python file for
trust information. It doesn't load any code from that folder.

## Unsupported processors

Russet doesn't run Python, so it can't run custom processors that recipe
repositories supply as Python files. It also doesn't recognize any
`RECIPE_IDENTIFIER/PROCESSOR` name other than the alias in the previous
section.

When you run `russet run` or `russet install`, Russet checks every
processor in every recipe that you named, including preprocessors and
postprocessors, before it runs any recipe. If a recipe uses a processor that
Russet doesn't implement, Russet prints the following error and exits with
status `1` without running any recipe:

```text
Custom or unknown processor 'NAME' is not supported
```

To run the other recipes, remove the recipe that uses the unsupported
processor from the command or the recipe list. For more about what Russet
supports, see [Compatibility with AutoPkg](/concepts/compatibility/).

## Related pages

- [`russet list-processors`](/reference/cli/russet-list-processors/)
- [`russet processor-info`](/reference/cli/russet-processor-info/)
- [Platform support](/concepts/platform-support/)
- [Compatibility with AutoPkg](/concepts/compatibility/)
