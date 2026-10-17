---
title: russet repo-add
description: Clone recipe repositories and add them to your recipe search path.
---

`russet repo-add` clones one or more recipe repositories with Git and adds
them to your recipe search folders.

## Syntax

```sh
russet repo-add [OPTIONS] REPOSITORY [REPOSITORY ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `REPOSITORY`: the recipe repository to add, in one of these forms:
  - `NAME`, such as `recipes`, for `https://github.com/autopkg/NAME`.
  - `OWNER/NAME` for `https://github.com/OWNER/NAME`.
  - A Git URL that starts with `https://`, `http://`, `git://`, or `ssh://`.
  - An SSH address such as `git@example.com:team/recipes`.

## Description

For each recipe repository, Russet does the following:

1. Clones the recipe repository into `RECIPE_REPO_DIR`, in a folder named
   after the reversed host name and path, such as
   `com.github.autopkg.recipes`. If the folder already exists, Russet runs
   `git pull` in it instead.
1. Adds the recipe repository to the `RECIPE_REPOS` preference and its folder
   to the `RECIPE_SEARCH_DIRS` preference.
1. Saves your preferences, refreshes the recipe map, and prints the updated
   search path.

Russet runs the Git executable that the `GIT_PATH` preference names, or `git`
from your `PATH`. If Git fails, Russet prints the error from Git and continues, so
check the output.

Russet doesn't accept `file://` URLs.

Russet saves the changed preferences to Russet's own preferences, or to the
file that you pass with `--prefs`. It never changes Python AutoPkg's
preferences. For details, see
[Preferences](/reference/preferences/#where-russet-saves-changes).

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` on top of Russet's preferences and saves the changed keys to `FILE`. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet finished. Git errors don't change the status. |
| `1` | Russet couldn't save your preferences. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name a recipe repository. On Windows, the status is `-1`. |

## Examples

To add the AutoPkg project's main recipe repository, run the following
command:

```sh
russet repo-add recipes
```

The output looks like the following:

```text
Attempting git clone for https://github.com/autopkg/recipes...

Adding /Users/alex/Library/Russet/RecipeRepos/com.github.autopkg.recipes to RECIPE_SEARCH_DIRS...
Updated search path:
  '.'
  '~/Library/Russet/Recipes'
  '/Library/Russet/Recipes'
  '/Users/alex/Library/Russet/RecipeRepos/com.github.autopkg.recipes'
```

To add a recipe repository from another GitHub account, run the following
command:

```sh
russet repo-add OWNER/NAME
```

Replace the following:

- `OWNER`: the GitHub user or organization.
- `NAME`: the recipe repository name.

## Related pages

- [Add recipe repositories](/guides/add-recipe-repositories/)
- [`russet repo-update`](/reference/cli/russet-repo-update/)
- [`russet repo-delete`](/reference/cli/russet-repo-delete/)
