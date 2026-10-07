---
title: autopkg search
description: Search for recipes in the AutoPkg organization on GitHub.
---

`autopkg search` searches an index of the recipes in the AutoPkg organization
on GitHub and prints the matches.

## Syntax

```sh
autopkg search [OPTIONS] TERM
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `TERM`: the text to search for, such as an app name. Russet uses only the
  first search term.

## Description

Russet downloads the recipe index from GitHub with curl and caches it in
`CACHE_DIR/search_index.json`. If the download fails, Russet uses the cached
index.

Russet matches `TERM` against recipe names, app names, and paths. Matching
ignores letter case, spaces, periods, commas, and hyphens. Russet leaves out
deprecated recipes.

Russet prints up to 100 matches in a table with `Name`, `Repo`, and `Path`
columns, sorted by recipe repository. After the table, it prints a reminder
of how to add a recipe repository. If nothing matches, Russet prints
`Nothing found.` to standard error.

If you set the `GITHUB_TOKEN` preference or store a token in the file that
`GITHUB_TOKEN_PATH` names, Russet uses the token for GitHub requests. Russet
ignores a token that contains spaces and prints a warning.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-p`, `--path-only` | Matches `TERM` against recipe paths only. Use it to find recipes in a specific folder or recipe repository. |
| `-u ORG`, `--user ORG` | Deprecated. Prints a GitHub code search URL for `ORG` instead of searching. |
| `-t`, `--use-token` | Deprecated. Russet prints a warning and ignores this option. Russet uses a configured token automatically. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | The search finished. Russet returns this status whether or not it found matches. |
| `1` | You didn't give a search term, or Russet couldn't download the index and has no cached copy. |
| `2` | Russet couldn't parse an option. |

## Examples

To search for recipes for Firefox, run the following command:

```sh
autopkg search firefox
```

To find recipes whose path contains `Mozilla`, run the following command:

```sh
autopkg search --path-only Mozilla
```

## Related pages

- [Add recipe repositories](/guides/add-recipe-repositories/)
- [`autopkg repo-add`](/reference/cli/autopkg-repo-add/)
- [Preferences](/reference/preferences/)
