---
title: autopkg make-override
description: Create a recipe override with trust information for its parent recipes.
---

`autopkg make-override` creates a recipe override: a small recipe that
inherits from another recipe, lets you change its input values, and records
trust information for its parent recipes.

## Syntax

```sh
autopkg make-override [OPTIONS] RECIPE
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `RECIPE`: the name or identifier of the recipe to override.

## Description

Russet looks for `RECIPE` at the top level of each search and override folder
and one folder below it, so you can name a recipe from a recipe repository by
its short name. You can't pass a path to a recipe file.

Russet writes the override to the first folder in `RECIPE_OVERRIDE_DIRS`, or
to the first `--override-dir` folder. The filename is the recipe name with a
`.recipe` extension, or `.recipe.yaml` for the YAML format. The override
contains the following keys:

- `Identifier`: `local.` followed by the parts of the recipe name in reverse
  order. For example, the override for `TheUnarchiver.download` has the
  identifier `local.download.TheUnarchiver`.
- `Input`: the input values that the recipe inherits, except `IDENTIFIER`.
  Edit these values to customize the recipe.
- `ParentRecipe`: the identifier of the recipe that you named.
- `ParentRecipeTrustInfo`: hashes of the parent recipes and of any
  non-built-in processor files, which Russet checks before each run.

Russet refuses to overwrite an existing override unless you add `--force`,
and refuses to override a deprecated recipe unless you add
`--ignore-deprecation`. If the override is inside your override folders,
Russet refreshes the recipe map.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for the recipe instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Writes the override to `FOLDER` instead of the first folder in `RECIPE_OVERRIDE_DIRS`. If you repeat this option, Russet uses the first folder. |
| `-n FILENAME`, `--name FILENAME` | Names the override file `FILENAME`. The name can't contain path separators. |
| `-f`, `--force` | Overwrites an existing override file. |
| `-p`, `--pull` | Not supported. Russet exits with an error. To get a missing parent recipe, add its recipe repository with `autopkg repo-add`. |
| `--ignore-deprecation` | Creates the override even if the recipe or one of its parents has a `DeprecationWarning` step. |
| `--format FORMAT` | Writes the override as `plist` or `yaml`. The default is the value of the `RECIPE_OVERRIDE_FORMAT` preference, or `plist`. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet saved the override. |
| `1` | Russet couldn't find the recipe, the override already exists, you named a deprecated recipe, you passed a path, or you used `--pull`. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name exactly one recipe. On Windows, the status is `-1`. |

## Examples

To create an override for a recipe from a recipe repository, run the
following command:

```sh
autopkg make-override TheUnarchiver.download
```

The output looks like the following:

```text
Override file saved to /Users/alex/Library/AutoPkg/RecipeOverrides/TheUnarchiver.download.recipe
```

To create a YAML override, run the following command:

```sh
autopkg make-override --format yaml TheUnarchiver.download
```

## Related pages

- [Create recipe overrides](/guides/create-overrides/)
- [Recipe trust](/concepts/recipe-trust/)
- [`autopkg update-trust-info`](/reference/cli/autopkg-update-trust-info/)
