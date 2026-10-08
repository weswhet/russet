---
title: russet list-recipes
description: List the recipes in your search and override folders.
---

`russet list-recipes` lists the recipes that Russet finds in your recipe
search folders and override folders.

## Syntax

```sh
russet list-recipes [OPTIONS]
```

Replace `OPTIONS` with any of the options in the following table.

## Description

Russet looks for recipes at the top level of each search folder and one
folder below it, and at the top level of each override folder. By default,
it prints one recipe name for each line and leaves out repeated names. When an
override and its parent recipe have the same name, Russet lists only the
override unless you add `--show-all`.

With `--with-identifiers` or `--with-paths`, Russet prints aligned columns
and shows your home folder as `~`. With `--plist`, Russet prints an XML
property list array that contains every recipe's contents plus `Name`, `Path`,
and `IsOverride` keys.

`russet list-recipes` doesn't accept recipe arguments.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-i`, `--with-identifiers` | Adds each recipe's identifier. |
| `-p`, `--with-paths` | Adds each recipe's path. |
| `--plist` | Prints the list as a property list with every key of each recipe. You can't combine this option with `-i` or `-p`. |
| `-a`, `--show-all` | Includes recipes with repeated names. For example, it lists both a parent recipe and its override. Use it with `-i`, `-p`, or `--plist`. |
| `-d FOLDER`, `--search-dir FOLDER` | Lists recipes in `FOLDER` instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Lists overrides in `FOLDER` instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the list. |
| `1` | You passed a recipe argument, used `--show-all` without `-i`, `-p`, or `--plist`, or combined `--plist` with `-i` or `-p`. |
| `2` | Russet couldn't parse an option. |

## Examples

To list recipe names, run the following command:

```sh
russet list-recipes
```

To list recipe names with their identifiers, run the following command:

```sh
russet list-recipes --with-identifiers
```

The output looks like the following:

```text
Adium.download                        com.github.autopkg.download.Adium
Adium.install                         com.github.autopkg.install.Adium
Adium.munki                           com.github.autopkg.munki.Adium
```

## Related pages

- [`russet info`](/reference/cli/russet-info/)
- [`russet search`](/reference/cli/russet-search/)
- [Add recipe repositories](/guides/add-recipe-repositories/)
