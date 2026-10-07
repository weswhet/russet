---
title: Add recipe repositories
description: Find recipes, add the Git repositories that contain them, and keep those repositories up to date.
---

This page shows you how to find recipes, add the recipe repositories that
contain them, list the recipes that Russet can find, and update or remove
repositories.

## Before you begin

- [Install Russet](/get-started/install/).
- Make sure that you have Git. Russet runs the `git` command from your
  `PATH` unless the `GIT_PATH` preference names another one.
- On Linux and Windows, create a preference file so that Russet can save the
  repositories that you add. For details, see
  [Configure preferences](/guides/configure-preferences/).

## Search for recipes

`autopkg search` searches an index of the recipes in the AutoPkg organization
on GitHub. To search for recipes, run the following command:

```sh
autopkg search SEARCH_TERM
```

Replace `SEARCH_TERM` with part of an app name, recipe name, or path, such as
`firefox`. The match ignores case, spaces, periods, commas, and hyphens.

The output lists each matching recipe with the recipe repository that
contains it. Russet caches the index in your cache folder and uses a GitHub
token if you configured one. For details, see
[Preferences](/reference/preferences/).

## Add a recipe repository

To add a recipe repository, run `autopkg repo-add` with the repository's
location:

```sh
autopkg repo-add REPOSITORY
```

Replace `REPOSITORY` with one of the following forms:

- A repository name in the AutoPkg organization, such as `recipes`.
- A GitHub user or organization and a repository name, such as
  `OWNER/REPOSITORY_NAME`.
- A full Git URL that starts with `https://`, `git://`, or `ssh://`, or an
  SSH location such as `git@github.com:OWNER/REPOSITORY_NAME.git`.

Russet clones the repository into your recipe repository folder, adds it to
the `RECIPE_SEARCH_DIRS` preference, and prints the updated search path. If
the repository is already present, Russet updates it with `git pull` instead.

:::caution
If Git can't clone the repository, Russet prints the error from Git and
still exits with status `0`. Check the output, or run `autopkg repo-list` to
confirm that Russet added the repository.
:::

Russet doesn't accept `file://` URLs.

## List repositories and recipes

To list the recipe repositories that you added, run the following command:

```sh
autopkg repo-list
```

Each line shows the local path and the URL of one repository.

To list the recipes that Russet can find, run the following command:

```sh
autopkg list-recipes
```

To include identifiers and paths, add `--with-identifiers` and
`--with-paths`. When an override and its parent recipe have the same short
name, the list shows only the override unless you add `--show-all`.

To see a recipe's description, input variables, and parent recipes, run the
following command:

```sh
autopkg info RECIPE
```

Replace `RECIPE` with a recipe's short name, identifier, or path.

## Update recipe repositories

Recipe authors fix and improve recipes over time. To update every recipe
repository that you added, run the following command:

```sh
autopkg repo-update all
```

To update one repository, replace `all` with the repository's name, URL, or
local path. After you update repositories, verify the trust information of
your overrides. For details, see
[Create recipe overrides](/guides/create-overrides/).

## Remove a recipe repository

To remove a recipe repository, run the following command:

```sh
autopkg repo-delete REPOSITORY
```

Replace `REPOSITORY` with the form that you used to add it, such as
`recipes`, or with its local path. Russet removes the repository from your
preferences and then deletes its folder.

:::caution
`autopkg repo-delete` deletes the repository's local folder, including any
changes that you made in it.
:::

## What's next

- [Run recipes](/guides/run-recipes/)
- [Create recipe overrides](/guides/create-overrides/)
- [Audit recipes](/guides/audit-recipes/)
