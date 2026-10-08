---
title: russet repo-list
description: List the recipe repositories that you added.
---

`russet repo-list` lists the recipe repositories that you added with
`russet repo-add`. The alias `russet list-repos` does the same thing.

## Syntax

```sh
russet repo-list [OPTIONS]
```

Replace `OPTIONS` with any of the options in the following table.

## Description

Russet reads the `RECIPE_REPOS` preference and prints one line for each
recipe repository, in the form `PATH (URL)`, sorted by path. If you haven't
added any recipe repositories, Russet prints `No recipe repos.`

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet printed the list. |
| `1` | Russet couldn't read the `--prefs` file. |
| `2` | Russet couldn't parse an option. |

## Examples

To list your recipe repositories, run the following command:

```sh
russet repo-list
```

The output looks like the following:

```text
/Users/alex/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes (https://github.com/autopkg/recipes)
```

## Related pages

- [`russet repo-add`](/reference/cli/russet-repo-add/)
- [`russet repo-update`](/reference/cli/russet-repo-update/)
- [`russet repo-delete`](/reference/cli/russet-repo-delete/)
