---
title: How Russet works
description: How the autopkg command, the recipe engine, the built-in processors, and the macOS helper services work together.
---

Russet is a native implementation of AutoPkg's recipe engine, command-line
interface, and built-in processors. It runs the recipes that you already use,
with the same command name, preference keys, and file locations, but it
doesn't run Python. This page explains the parts of Russet and how a recipe
run uses them.

## The parts of Russet

A Russet installation has the following parts:

- **The `autopkg` command:** a single native executable for macOS, Linux, or
  Windows. It reads your preferences, finds recipes, and runs them.
- **The recipe engine:** loads a recipe and its parent recipes, merges their
  input variables, checks trust information, and runs each processor in
  order.
- **Built-in processors:** native implementations of the processors that
  recipes name in their `Process` steps, such as `URLDownloader`,
  `CodeSignatureVerifier`, and `MunkiImporter`. For the full list, see
  [Processors](/reference/processors/).
- **Helper services:** on macOS, two privileged launchd services that
  build packages and install software for recipes that run as a standard
  user.

## What happens when you run a recipe

When you run `autopkg run`, Russet does the following:

1. Reads your preferences and any `--prefs` file.
1. Finds each recipe that you named, searching your override folders first
   and then your recipe search folders.
1. Loads each recipe's parent recipes and checks the `MinimumVersion` of every
   recipe in the chain.
1. Verifies trust information for recipe overrides.
1. Checks every processor that the recipes use. If any recipe names a
   processor that Russet doesn't implement, Russet stops before it runs any
   recipe.
1. Runs each recipe's processors in order. Each processor reads variables from
   the recipe environment and adds its results to it.
1. Writes a receipt for each recipe and records the run's results in the
   cache.

Because Russet checks every recipe before it runs any of them, an
unsupported recipe doesn't leave a run half finished. For details, see
[Compatibility with AutoPkg](/concepts/compatibility/).

## Helper services

On macOS, Russet needs root privileges to build a package or to install
software.
Russet uses the same design as Python AutoPkg: two launchd services run as
root, and recipes ask them to do privileged work over a local socket.

| Service | Socket | Used by |
| --- | --- | --- |
| `autopkgserver` | `/var/run/autopkgserver` | `PkgCreator`, `AppPkgCreator` |
| `autopkginstalld` | `/var/run/autopkginstalld` | `Installer`, `InstallFromDMG` |

launchd starts each service when a recipe connects to its socket, and the
service exits after it's idle for 10 seconds. The services check who sent each
request:

- The package builder accepts a request only if the requesting user owns the
  package root and the output folder.
- The package installer installs only packages in the recipe's cache or on a
  volume mounted under `/private/tmp`, where Russet mounts disk images.

The Russet services replace Python AutoPkg's services under the same names
and socket paths. On Linux and Windows, the processors that need them report
that only macOS supports package creation and installation.

## Native tools

Russet runs operating system tools where AutoPkg does:

- **Downloads:** Russet runs curl for every download and web request.
- **Recipe repositories:** Russet runs Git to clone and update recipe
  repositories.
- **macOS operations:** Russet runs tools such as `hdiutil` to mount disk
  images, `pkgutil` to expand packages, and `codesign` to verify signatures.
- **Windows operations:** Russet runs `choco.exe` to build Chocolatey
  packages and `signtool.exe` to verify Authenticode signatures.

Russet doesn't run `makepkginfo` or any other Munki tool. It generates Munki
metadata natively. For details, see
[Import software into Munki](/guides/import-into-munki/).

## Versions

Russet has two version numbers:

- The **distribution version** identifies a Russet release, such as 0.1.0.
  Each release archive records it in `RELEASE.json`. An archive that you
  build from source doesn't have a distribution version.
- The **compatibility version** is the AutoPkg version that Russet
  implements. `autopkg version` prints it, and recipes receive it in the
  `AUTOPKG_VERSION` variable. Russet's compatibility version is 3.0.0.

## What's next

- [Compatibility with AutoPkg](/concepts/compatibility/)
- [Platform support](/concepts/platform-support/)
- [Recipe trust](/concepts/recipe-trust/)
