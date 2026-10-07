---
title: autopkg verify-trust-info
description: Check that a recipe override's parent recipes haven't changed since you trusted them.
---

`autopkg verify-trust-info` checks the trust information in recipe overrides
against their parent recipes and reports any differences.

## Syntax

```sh
autopkg verify-trust-info [OPTIONS] OVERRIDE [OVERRIDE ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `OVERRIDE`: the name or path of a recipe override.

## Description

For each override, Russet compares the hashes in its `ParentRecipeTrustInfo`
key with the current parent recipes and their non-built-in processor files.
Russet prints `NAME: OK` to standard output when they match, and
`NAME: FAILED` to standard error when they don't. With `--verbose`, Russet
also prints the reason for each failure.

The override must be in one of your override folders and outside your recipe
repositories. Otherwise, verification fails with
`Trust records must come from a local override outside recipe repositories`.
For details, see [Recipe trust](/concepts/recipe-trust/).

A recipe list that you pass with `--recipe-list` can be a property list with
a `recipes` array, or a text file with one recipe for each line. In a text
file, Russet removes spaces from the start and end of each line and skips
lines that start with `#`.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-l FILE`, `--recipe-list FILE` | Verifies the overrides in the recipe list `FILE`. |
| `-v`, `--verbose` | Prints the reason for each failure. Repeating this option doesn't add more detail. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for parent recipes instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for recipe overrides instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Every override passed. |
| `1` | At least one override failed verification. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name a recipe override. On Windows, the status is `-1`. |

## Examples

To verify an override and print the reason if it fails, run the following
command:

```sh
autopkg verify-trust-info -v TheUnarchiver.download
```

When the parent recipes haven't changed, the output is the following:

```text
TheUnarchiver.download: OK
```

## Related pages

- [Recipe trust](/concepts/recipe-trust/)
- [`autopkg update-trust-info`](/reference/cli/autopkg-update-trust-info/)
- [Create recipe overrides](/guides/create-overrides/)
