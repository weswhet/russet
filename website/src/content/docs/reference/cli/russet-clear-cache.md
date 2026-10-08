---
title: russet clear-cache
description: Remove cached files for one recipe or for all recipes.
---

`russet clear-cache` removes a recipe's cache folder, or everything in the
cache.

## Syntax

```sh
russet clear-cache [OPTIONS] TARGET
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `TARGET`: `all` to remove everything in the cache, or a recipe name,
  identifier, or path to remove that recipe's cache folder.

## Description

With `all`, Russet removes every item in the cache folder and prints
`Removing all cached items from CACHE_DIR`.

With a recipe, Russet removes `CACHE_DIR/IDENTIFIER`, where `IDENTIFIER` is
the recipe's identifier. Russet looks for the recipe only in the current
folder and in the folders that you pass with `--search-dir` or
`--override-dir`. It doesn't search the folders in your preferences, so pass
the recipe's folder or its path.

Russet removes symbolic links without following them.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `--dry-run` | Prints what Russet would remove without removing anything. |
| `-v`, `--verbose` | With `all`, lists each top-level cache item. With `-vv`, also lists the items inside each removed folder. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for the recipe. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for the recipe. Russet treats it the same way as `--search-dir`. You can repeat this option. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet removed the items, or printed them with `--dry-run`. |
| `1` | You didn't pass exactly one target, Russet couldn't find the recipe or its cache folder, the cache folder doesn't exist, or Russet couldn't remove an item. |
| `2` | Russet couldn't parse an option. |

## Examples

To see what clearing the whole cache would remove, run the following command:

```sh
russet clear-cache --dry-run all
```

To remove the cache of a recipe override in the default override folder, run
the following command:

```sh
russet clear-cache --override-dir ~/Library/AutoPkg/RecipeOverrides TheUnarchiver.download
```

## Related pages

- [Files and paths](/reference/files-and-paths/)
- [`russet run`](/reference/cli/russet-run/)
