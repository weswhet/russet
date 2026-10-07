---
title: Import software into Munki
description: Point Russet at a file-based Munki repository, run Munki recipes, and rebuild catalogs.
---

This page shows you how to set up Russet to import software into a Munki
repository, run Munki recipes, and rebuild Munki's catalogs after an import.

## Before you begin

- Use macOS or Linux. Russet reads packages and disk images with macOS tools
  on macOS and with built-in readers on Linux.
- Make sure that you have a file-based Munki repository with `pkgs`,
  `pkgsinfo`, and `catalogs` folders. If the repository is on a file share,
  mount the share first.
- [Add the recipe repositories](/guides/add-recipe-repositories/) that
  contain your Munki recipes, and
  [create overrides](/guides/create-overrides/) for them.

You don't need to install the Munki tools on the computer that runs Russet.
Russet generates pkginfo files natively, compatible with Munki
<!-- vale Google.DateFormat = NO -->7.2.0.5787<!-- vale Google.DateFormat = YES -->,
and never runs `makepkginfo` or `munkiimport`.

## Set the Munki repository

Russet supports only Munki's `FileRepo` backend, so the repository must be a
local path or a `file://` URL.

To set the repository for every run, run the following command:

```sh
defaults write com.github.autopkg MUNKI_REPO MUNKI_REPO_PATH
```

Replace `MUNKI_REPO_PATH` with the absolute path to the repository, such as
`/Users/Shared/munki_repo`.

To use a different repository for one run, add `-k MUNKI_REPO=MUNKI_REPO_PATH`
to `russet run` instead.

:::caution
Don't set `MUNKI_REPO_PLUGIN` to a backend other than `FileRepo`, and don't
set `MUNKI_REPO` to a URL other than a `file://` URL. If you do, Russet rejects
every recipe that you run, including recipes that don't use Munki.
:::

## Choose catalogs

Munki recipes usually import new items into the `testing` catalog. To use
another catalog for one recipe, set the `catalogs` key in the `pkginfo`
dictionary of the recipe's override. For details, see
[Create recipe overrides](/guides/create-overrides/#change-input-values).

Recipes that use the `MunkiSetDefaultCatalog` processor read the
`default_catalog` key from the `com.googlecode.munki.munkiimport` preference
domain, which `munkiimport --configure` sets.

## Import software

To run a Munki recipe and rebuild the catalogs, run the recipe and the
`MakeCatalogs.munki` recipe in the same command:

```sh
russet run RECIPE MakeCatalogs.munki
```

Replace `RECIPE` with the name of your Munki recipe's override, such as
`Firefox.munki`. Create an override for `MakeCatalogs.munki` too, because it's
a recipe in the `autopkg/recipes` repository.

Russet imports the software and prints a summary like the following:

```text
The following new items were imported into Munki:
    Name           Version  Catalogs  Pkginfo Path                    Pkg Repo Path
    ----           -------  --------  ------------                    -------------
    TheUnarchiver  4.3.9    testing   apps/TheUnarchiver-4.3.9.plist  apps/TheUnarchiver-4.3.9.dmg
```

For each import, Russet does the following:

- Copies the installer item to `pkgs`, in the subfolder that the recipe names.
- Writes a pkginfo file to the same subfolder of `pkgsinfo`.
- Skips the import if the repository's `all` catalog already has a matching
  item. To import it anyway, set the `force_munkiimport` input variable.

## Rebuild catalogs

The `MakeCatalogs.munki` recipe uses the `MakeCatalogsProcessor` processor,
which Russet implements natively. It rebuilds the catalogs only when a recipe
earlier in the same command imported something, so put it last in the
command or recipe list. Otherwise, it prints `No need to rebuild catalogs.`

To rebuild the catalogs even when no recipe imported anything, add
`-k force_rebuild=1`. Any non-empty value turns on a forced rebuild.

## Use Munki recipes on Linux and Windows

On Linux, Russet reads packages and disk images itself, so recipes that import
them into Munki work, including `extract_icon`. The `installerChoices` option
still needs macOS. On Linux, icon PNG files aren't byte-identical to the
ones macOS writes, though the pixels match. On Windows, Russet can't read packages or
disk images, so those recipes fail. On every platform, processors that only
edit pkginfo data, such as `MunkiPkginfoMerger`, and `MakeCatalogsProcessor`
work.

## What's next

- [Run recipes](/guides/run-recipes/)
- [Schedule recipe runs](/guides/schedule-runs/)
- [Processors](/reference/processors/)
