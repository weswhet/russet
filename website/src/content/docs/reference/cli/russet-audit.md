---
title: russet audit
description: Check recipes for risky patterns without running them.
---

`russet audit` reads recipes and reports patterns that deserve review, such
as insecure downloads or missing code signature checks. It doesn't run any
processors.

## Syntax

```sh
russet audit [OPTIONS] RECIPE [RECIPE ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `RECIPE`: a recipe name, recipe identifier, or path to a recipe file.

## Description

`russet audit` runs the following checks on each recipe:

| Check | Severity | What it reports |
| --- | --- | --- |
| `insecure_protocol` | Warning | URLs that use HTTP instead of HTTPS. |
| `missing_codesig` | Warning | Recipes that don't use `CodeSignatureVerifier`. |
| `modification_processor` | Info | Processors that make changes, such as editing files, whose use deserves a closer look. |
| `non_core_processor` | Info | Processors that aren't built-in processors and that could run any code. |
| `path_safety` | Warning | Paths with parent folder references, and privileged installation paths outside the cache or a mounted disk image. |
| `sensitive_input` | Error | Input values that look like credentials. Russet never prints the values. |
| `weak_hash` | Warning | Checksums that use a weak algorithm, such as MD5. |

By default, Russet prints a block of findings for each recipe, or
`NAME: no audit flags triggered.` when a recipe has no findings. When you
audit more than one recipe, Russet also prints a summary.

If Russet can't find a recipe, it prints an error to standard error and
continues with the next recipe.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for recipes instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for recipe overrides instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |
| `-l FILE`, `--recipe-list FILE` | Audits the recipes in the recipe list `FILE`. |
| `-p`, `--plist` | Prints the findings as a property list dictionary keyed by recipe. |
| `-j`, `--json` | Prints the findings as JSON, with a severity for each finding. You can't combine this option with `--plist`. |
| `--only-check CHECKS` | Reports only the checks in `CHECKS`, a comma-separated list of check names. |
| `--skip-check CHECKS` | Leaves out the checks in `CHECKS`. You can't combine this option with `--only-check`. |
| `--list-checks` | Prints each check name with its severity, and then exits. |
| `--fail-on SEVERITY` | Exits with status `1` if any finding has severity `SEVERITY` or higher. `SEVERITY` is `info`, `warning`, or `error`. |

The JSON output is an array with one object for each recipe. Each object has a
`recipe` key and a `findings` array, and each finding has `check`, `detail`,
and `severity` keys.

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | The audit finished. Recipes that Russet couldn't find don't change the status. |
| `1` | A finding reached the `--fail-on` severity, you combined options that don't work together, or you named an unknown check or severity. |
| `2` | Russet couldn't parse an option. |
| `255` | You didn't name any recipes. On Windows, the status is `-1`. |

## Examples

To audit a recipe, run the following command:

```sh
russet audit TheUnarchiver.download
```

The output is the following:

```text
TheUnarchiver.download: no audit flags triggered.
```

To fail a continuous integration job when any recipe in a list has a
warning or an error, run the following command:

```sh
russet audit --json --fail-on warning --recipe-list recipes.txt
```

## Related pages

- [Audit recipes](/guides/audit-recipes/)
- [Recipe trust](/concepts/recipe-trust/)
