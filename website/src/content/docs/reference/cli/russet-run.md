---
title: russet run
description: Run one or more recipes.
---

`russet run` runs one or more recipes and prints a summary of what they
downloaded, built, and imported.

## Syntax

```sh
russet run [OPTIONS] [RECIPE ...]
```

Replace the following:

- `OPTIONS`: any of the options in the following table.
- `RECIPE`: a recipe override name, a recipe identifier, or a path to a recipe
  file. You can name more than one recipe.

## Description

`russet run` works in two stages:

1. **Validation:** Russet finds every recipe that you named, loads its parent
   recipes, verifies trust information for recipe overrides, and checks
   every processor that the recipes use. If any recipe fails this stage,
   Russet stops with exit status `1` before it runs any recipe, and it
   doesn't write a report.
1. **Execution:** Russet runs each recipe in order and prints
   `Processing NAME...` for each one. If a processor fails, Russet prints the
   error and `Failed.`, and then continues with the next recipe.

When every recipe has run, Russet prints the recipes that failed, if any, and
a summary of the items that the recipes downloaded, built, or imported. If
nothing changed, it prints `Nothing downloaded, packaged or imported.`

The following problems stop the validation stage:

- Russet can't find a recipe or one of its parent recipes.
- A recipe override's trust information doesn't match its parent recipes.
- A recipe override has no trust information and the
  `FAIL_RECIPES_WITHOUT_TRUST_INFO` preference is `true`.
- A recipe, preprocessor, or postprocessor names a processor that Russet
  doesn't implement.
- A recipe uses a Munki repository backend other than `FileRepo`.
- With `--check`, a recipe has no `EndOfCheckPhase` step.

### How Russet finds recipes

For each `RECIPE`, Russet does the following:

1. If `RECIPE` is the path of an existing file, Russet uses that file.
1. Otherwise, Russet looks in the current folder, then in each override
   folder, and then in each search folder for a file named `RECIPE`,
   `RECIPE.recipe`, `RECIPE.recipe.yaml`, or `RECIPE.recipe.plist`. In each
   folder, it checks the top level and then the folders one level down.
1. Otherwise, Russet looks for a recipe whose `Identifier` is `RECIPE` in the
   same folders and the same two levels.

Russet skips symbolic links below the top level of a folder. An override has
the same short name as its parent recipe, and Russet searches override
folders before search folders, so the override runs. If Russet can't find a
recipe, it prints `Could not find parent recipe RECIPE`.

Russet doesn't accept trust information from an override inside a recipe
repository: the `RECIPE_REPO_DIR` folder or a repository in `RECIPE_REPOS`.
For details, see
[Recipe trust](/concepts/recipe-trust/#where-trusted-overrides-must-be).

### Recipe variables

Russet builds each recipe's variables from the following sources. A later
source overrides an earlier one:

1. Your preferences.
1. The recipe's `Input` dictionary, including values that it inherits from
   parent recipes.
1. Environment variables whose names start with `AUTOPKG_`, without the
   prefix.
1. Keys in a property list recipe list.
1. Values from `--key`.
1. The path from `--pkg`, as the `PKG` variable.

Russet also sets `AUTOPKG_VERSION` to `3.0.0`.

### Recipe lists

A recipe list that you pass with `--recipe-list` can be in one of these
formats:

- **Text:** one recipe per line. Russet skips empty lines and lines that
  start with `#`. It doesn't remove spaces from the start or end of a line.
- **Property list:** a dictionary with a `recipes` array and optional
  `preprocessors` and `postprocessors` arrays. Russet passes every other key
  to the recipes as a variable.

Preprocessors and postprocessors from `--pre` and `--post` replace the ones in
a property list recipe list.

### Files that Russet writes

After each recipe, Russet writes a receipt to
`RECIPE_CACHE_DIR/receipts/NAME-receipt-YYYYMMDD-HHMMSS.plist` and updates
`CACHE_DIR/autopkg_results.plist`. Receipts don't include the `GITHUB_TOKEN`
value. With `--report-plist`, Russet also writes a report when the run
finishes.

### Report property list

The report is an XML property list with these keys:

- `failures`: an array with one dictionary for each failed recipe. Each
  dictionary has `recipe`, `recipe_id`, `message`, and `traceback` keys. The
  `traceback` value contains the error message and a native backtrace.
- `summary_results`: a dictionary of summary tables, such as the items that
  `URLDownloader` downloaded and the items that `MunkiImporter` imported.

## Options

| Option | Description |
| --- | --- |
| `--prefs FILE` | Reads preferences from `FILE` in addition to your platform preferences. |
| `--pre NAME`, `--preprocessor NAME` | Runs the processor `NAME`, with no arguments, before each recipe's steps. You can repeat this option. |
| `--post NAME`, `--postprocessor NAME` | Runs the processor `NAME`, with no arguments, after each recipe's steps. You can repeat this option. With `--check`, Russet doesn't run postprocessors. |
| `-c`, `--check` | Runs each recipe only through its last `EndOfCheckPhase` step, which checks for new downloads without building or importing anything. |
| `--ignore-parent-trust-verification-errors` | Runs recipe overrides even if their trust information doesn't match their parent recipes. This option also ignores `FAIL_RECIPES_WITHOUT_TRUST_INFO`. |
| `-k KEY=VALUE`, `--key KEY=VALUE` | Sets the variable `KEY` to `VALUE` for every recipe in the run. `VALUE` is always a string. You can repeat this option. |
| `-l FILE`, `--recipe-list FILE` | Runs the recipes in the recipe list `FILE`. |
| `-p PATH`, `--pkg PATH` | Sets the `PKG` variable to a local package or disk image. You can use this option with only one recipe. |
| `--report-plist PATH` | Writes a report property list to `PATH` when the run finishes. |
| `-v`, `--verbose` | Prints more detail. You can repeat this option up to three times. |
| `-q`, `--quiet` | Accepted for compatibility. This option has no effect. |
| `-d FOLDER`, `--search-dir FOLDER` | Searches `FOLDER` for recipes instead of the folders in `RECIPE_SEARCH_DIRS`. You can repeat this option. |
| `--override-dir FOLDER` | Searches `FOLDER` for recipe overrides instead of the folders in `RECIPE_OVERRIDE_DIRS`. You can repeat this option. |

### Verbosity levels

Each `-v` adds more detail:

| Level | Adds |
| --- | --- |
| `-v` | The name of each processor, the messages that processors print, and the path of each receipt. |
| `-vv` | The `AUTOPKG_` environment variables that Russet applies, and the input and output of each step. |
| `-vvv` | The complete recipe environment before and after each step. |

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Every recipe ran without an error. |
| `1` | Validation failed, so no recipe ran. This status also covers a `--key` value without `=`, `--pkg` with more than one recipe, and a missing `--prefs` file. |
| `2` | Russet couldn't parse an option. |
| `70` | At least one recipe failed while it ran. Other recipes might have succeeded. |
| `255` | You didn't name any recipes. On Windows, the status is `-1`. |

## Examples

To run a recipe override named `TheUnarchiver.download` and print each
processor's messages, run the following command:

```sh
russet run -v TheUnarchiver.download
```

To run a recipe from a recipe repository by its identifier, run the following
command:

```sh
russet run com.github.autopkg.download.TheUnarchiver
```

To check the recipes in a recipe list for new downloads and save a report,
run the following command:

```sh
russet run --check --recipe-list ~/recipe-list.txt --report-plist ~/autopkg-report.plist
```

To set a variable for every recipe in a run, such as the Munki repository
path for a recipe override named `Firefox.munki`, run the following command:

```sh
russet run -k MUNKI_REPO=/Users/Shared/munki_repo Firefox.munki
```

## Related pages

- [Run recipes](/guides/run-recipes/)
- [Create recipe overrides](/guides/create-overrides/)
- [Recipe trust](/concepts/recipe-trust/)
- [`russet install`](/reference/cli/russet-install/)
- [Exit codes](/reference/exit-codes/)
