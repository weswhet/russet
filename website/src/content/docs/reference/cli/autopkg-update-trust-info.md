---
title: autopkg update-trust-info
description: Record the current state of a recipe override's parent recipes.
---

`autopkg update-trust-info` updates the trust information in recipe overrides
so that it matches the current state of their parent recipes.

## Syntax

```sh
autopkg update-trust-info [OPTIONS] OVERRIDE [OVERRIDE ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `OVERRIDE`: the name or path of a recipe override.

## Description

For each override, Russet computes new hashes for its parent recipes and for
any non-built-in processor files that they use, writes them to the
override's `ParentRecipeTrustInfo` key, and prints `Wrote updated PATH`.

Run this command only after you review the changes in the parent recipes. To
see what changed, run `autopkg verify-trust-info -v` first.

If a recipe isn't an override, Russet prints
`NAME is not a recipe override and has no parent recipe.` and skips it.

The override must be in one of your override folders and outside your recipe
repositories. For details, see [Recipe trust](/concepts/recipe-trust/).

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for parent recipes instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for recipe overrides instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet updated every override, or skipped recipes that aren't overrides. |
| `1` | Russet couldn't update at least one override. It prints `NAME: FAILED` and the reason. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name a recipe override. On Windows, the status is `-1`. |

## Examples

To update the trust information of an override after you review its parent
recipes, run the following command:

```sh
autopkg update-trust-info TheUnarchiver.download
```

## Related pages

- [Recipe trust](/concepts/recipe-trust/)
- [Create recipe overrides](/guides/create-overrides/)
- [`autopkg verify-trust-info`](/reference/cli/autopkg-verify-trust-info/)
