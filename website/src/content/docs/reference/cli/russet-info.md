---
title: russet info
description: Show your current preferences or details about a recipe.
---

`russet info` prints your current preferences, or a summary of one recipe.

## Syntax

```sh
russet info [OPTIONS] [RECIPE]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `RECIPE`: a recipe name, recipe identifier, or path to a recipe file.

## Description

Without a recipe, `russet info` prints `Current preferences:` followed by
your merged preferences.

With a recipe, `russet info` prints the following details:

- Description and identifier.
- Whether the recipe imports into Munki, has a check phase, and builds a
  package.
- The recipe's path and its parent recipes.
- The recipe's input values.

Russet looks for the recipe at the top level of each search and override
folder and one folder below it, so `russet info` finds recipes in recipe
repositories by name. If an override and its parent have the same name,
Russet shows the override.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for recipes instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for recipe overrides instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |
| `-q`, `--quiet` | Accepted for compatibility. This option has no effect. |
| `-p`, `--pull` | Not supported. Russet exits with an error. To get a missing parent recipe, add its recipe repository with `russet repo-add`. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the information. |
| `1` | Russet couldn't find the recipe, or you used `--pull`. |
| `2` | Russet couldn't parse an option. |
| `255` | You named more than one recipe. On Windows, the status is `-1`. |

## Examples

To show details about a recipe, run the following command:

```sh
russet info TheUnarchiver.download
```

The output looks like the following:

```text
Description:         Download recipe for The Unarchiver. Finds and downloads the latest 'The Unarchiver' release.
Identifier:          com.github.autopkg.download.TheUnarchiver
Munki import recipe: False
Has check phase:     True
Builds package:      False
Recipe file path:    /Users/alex/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes/The Unarchiver/TheUnarchiver.download.recipe
Input values:
 'NAME': 'TheUnarchiver', 'SPARKLE_FEED_URL': 'https://updates.devmate.com/com.macpaw.site.theunarchiver.xml'
```

To show your current preferences, run the following command:

```sh
russet info
```

## Related pages

- [`russet list-recipes`](/reference/cli/russet-list-recipes/)
- [Preferences](/reference/preferences/)
