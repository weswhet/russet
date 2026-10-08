---
title: russet generate-recipe-map
description: Build or rebuild the recipe map file.
---

`russet generate-recipe-map` builds the recipe map, a JSON file that lists
the recipes and overrides in your search and override folders.

## Syntax

```sh
russet generate-recipe-map [OPTIONS]
```

Replace `OPTIONS` with any of the options in the following table.

## Description

The recipe map is a JSON file with a `schema_version` of `1` and four
sections: `identifiers`, `shortnames`, `overrides`, and
`overrides-identifiers`. Russet writes it to the path in the
`AUTOPKG_RECIPE_MAP_PATH` environment variable, the `RECIPE_MAP_PATH`
preference, or `~/Library/AutoPkg/recipe_map.json`, in that order.

Russet also refreshes the map automatically after `repo-add`, after
`repo-update` changes a recipe repository, and after `make-override`. It
generates the map when a command looks up a recipe by name and the map is
missing. Run this command to rebuild the map on demand, such as in a
continuous integration job. Russet itself finds recipes by scanning your folders,
not by reading the map.

By default, Russet leaves the current folder, `.`, out of the map. If you turn
off the recipe map with the `DISABLE_RECIPE_MAP` preference or the
`AUTOPKG_DISABLE_RECIPE_MAP` environment variable, the command does nothing.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `--include-cwd` | Includes the current folder in the map. |
| `-d FOLDER`, `--search-dir FOLDER` | Not supported. Russet exits with an error. Set the `RECIPE_SEARCH_DIRS` preference instead. |
| `--override-dir FOLDER` | Not supported. Russet exits with an error. Set the `RECIPE_OVERRIDE_DIRS` preference instead. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet wrote the map, you turned off the map, or Russet couldn't write the map and printed a warning. |
| `1` | You used `--search-dir` or `--override-dir`. |
| `2` | Russet couldn't parse an option. |

## Examples

To rebuild the recipe map, run the following command:

```sh
russet generate-recipe-map
```

The output looks like the following:

```text
Recipe map written to /Users/alex/Library/AutoPkg/recipe_map.json:
  identifiers:           228
  shortnames:            228
  overrides:             1
  overrides-identifiers: 1
```

## Related pages

- [Preferences](/reference/preferences/)
- [Environment variables](/reference/environment-variables/)
- [Files and paths](/reference/files-and-paths/)
