---
title: autopkg repo-update
description: Update recipe repositories with Git.
---

`autopkg repo-update` updates one or more recipe repositories by running
`git pull` in each one.

## Syntax

```sh
autopkg repo-update [OPTIONS] REPOSITORY [REPOSITORY ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `REPOSITORY`: `all` to update every recipe repository that you added, or a
  recipe repository's local path, URL, or short form, such as `recipes`.

## Description

For each recipe repository, Russet prints
`Attempting git pull for PATH...` and then the output from Git. If the local branch
is `master` and the remote has only a `main` branch, Russet switches the local
branch to `main` before it pulls. If anything changed, Russet refreshes the
recipe map.

Russet runs the Git executable that the `GIT_PATH` preference names, or `git`
from your `PATH`. If Git fails or Russet can't find a recipe repository,
Russet prints the error and continues, so check the output.

After an update, recipe overrides whose parent recipes changed fail trust
verification until you review the changes. For details, see
[Recipe trust](/concepts/recipe-trust/).

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet finished. Git errors and unknown recipe repositories don't change the status. |
| `1` | Russet couldn't read the `--prefs` file. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name a recipe repository. On Windows, the status is `-1`. |

## Examples

To update every recipe repository that you added, run the following command:

```sh
autopkg repo-update all
```

When a recipe repository has no changes, the output looks like the
following:

```text
Attempting git pull for /Users/alex/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes...
Already up to date.
```

## Related pages

- [`autopkg repo-add`](/reference/cli/autopkg-repo-add/)
- [`autopkg verify-trust-info`](/reference/cli/autopkg-verify-trust-info/)
- [Add recipe repositories](/guides/add-recipe-repositories/)
