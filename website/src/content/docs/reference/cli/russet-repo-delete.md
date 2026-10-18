---
title: russet repo-delete
description: Remove recipe repositories and take them out of your recipe search path.
---

`russet repo-delete` removes recipe repositories from your preferences and
deletes their local folders.

## Syntax

```sh
russet repo-delete [OPTIONS] REPOSITORY [REPOSITORY ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `REPOSITORY`: a recipe repository's local path, its URL, or a short form
  that `russet repo-add` accepts, such as `recipes`.

## Description

For each recipe repository, Russet removes it from the `RECIPE_REPOS` and
`RECIPE_SEARCH_DIRS` preferences and then deletes its folder. Russet doesn't
run Git.

To protect your files, Russet refuses to delete a file system root, your home
folder, or a folder that's a symbolic link. If Russet can't delete the folder,
it prints an error that says that it removed the recipe repository from your
preferences, and you can delete the folder yourself. On Windows, read-only
files can stop the deletion.

If you name a recipe repository that you didn't add, Russet prints
`ERROR: Can't find an installed repo for REPOSITORY` and continues.

Russet saves the changed preferences to Russet's own preferences, or to the
file that you pass with `--prefs`. It never changes Python AutoPkg's
preferences, so a recipe repository that you remove from Russet stays in
Python AutoPkg's preferences.

When you don't pass `--prefs`, Russet also reads Python AutoPkg's
preferences. If they list the recipe repository, Russet keeps its folder so
that Python AutoPkg keeps working, and prints
`Leaving PATH in place because AutoPkg's preferences list it.` For details,
see [Preferences](/reference/preferences/#autopkgs-preferences).

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` on top of Russet's preferences and saves the changed keys to `FILE`. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet finished, including when it couldn't find a recipe repository. |
| `1` | Russet couldn't save your preferences. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name a recipe repository. On Windows, the status is `-1`. |

## Examples

To remove the AutoPkg project's main recipe repository, run the following
command:

```sh
russet repo-delete recipes
```

## Related pages

- [`russet repo-add`](/reference/cli/russet-repo-add/)
- [`russet repo-list`](/reference/cli/russet-repo-list/)
- [Add recipe repositories](/guides/add-recipe-repositories/)
