---
title: Run recipes
description: Run one or more recipes, check for new versions, override input values, and save a run report.
---

This page shows you how to run recipes with `russet run`, check for new
versions without building anything, change input values for a run, run a
list of recipes, and save a report.

## Before you begin

- [Add a recipe repository](/guides/add-recipe-repositories/).
- [Create an override](/guides/create-overrides/) for each recipe that you
  plan to run regularly.

## Name the recipes to run

`russet run` accepts recipes in the following forms:

- **A short name**, such as `Firefox.download`.
- **A recipe identifier**, such as `com.github.autopkg.download.firefox-rc-en_US`.
- **A path** to a recipe file.

To find a short name, Russet searches the current folder, then your override
folders, and then your recipe search folders. In each folder, it checks the
top level and then the folders one level down, which is where recipe
repositories keep their recipes. An override has the same short name as its
parent recipe, and Russet searches override folders first, so the override
runs. If Russet can't find a recipe, it prints
`Could not find parent recipe NAME`.

## Run a recipe

To run a recipe, run the following command:

```sh
russet run RECIPE
```

Replace `RECIPE` with an override name, an identifier, or a path. To run
several recipes, list them in the same command. Russet runs them in order.

When the run finishes, Russet prints a summary of what it downloaded,
packaged, and imported. If nothing changed, the summary is the following:

```text
Nothing downloaded, packaged or imported.
```

To see what each processor does, add `-v`. Add more `v` characters, such as
`-vv` or `-vvv`, to see each processor's input and output variables and the
full recipe environment.

:::note
Russet checks every recipe that you name before it runs any of them. If
Russet can't find a recipe, or a recipe fails trust verification or uses a
processor that Russet doesn't support, Russet runs none of the recipes and exits with status
`1`.
:::

## Check for new versions

Many recipes end their check phase with the `EndOfCheckPhase` processor, after
they download the software and before they package or import it. To run only
the check phase, add `--check`:

```sh
russet run --check RECIPE
```

Russet stops each recipe at its last `EndOfCheckPhase` step. If a recipe
doesn't have one, Russet reports an error and runs none of the recipes. In
check mode, Russet doesn't run postprocessors.

## Change input values for a run

To set an input variable for one run, use `--key`, or its short form, `-k`:

```sh
russet run -k KEY=VALUE RECIPE
```

Replace the following:

- `KEY`: the name of an input variable, such as `MUNKI_REPO`.
- `VALUE`: the value to use. Russet passes the value as a string.

The value applies to every recipe in the run. To set a value permanently,
edit the recipe's override or set a preference instead.

To give a recipe a package or disk image that you already downloaded, use
`--pkg`:

```sh
russet run --pkg PATH RECIPE
```

Replace `PATH` with the path to the package or disk image. You can use
`--pkg` with only one recipe at a time.

## Run a list of recipes

A recipe list is a text file with one recipe per line. Russet skips empty
lines and lines that start with `#`. For example:

```text
# Recipes for the nightly run
Firefox.munki
GoogleChrome.munki
MakeCatalogs.munki
```

To run a recipe list, run the following command:

```sh
russet run --recipe-list RECIPE_LIST
```

Replace `RECIPE_LIST` with the path to the file.

A recipe list can also be a property list with a `recipes` array. In a
property list, the optional `preprocessors` and `postprocessors` arrays add
processors to every recipe, and any other keys become input variables.

## Add processors before or after each recipe

To run a built-in processor before or after every recipe in the run, use
`--preprocessor` or `--postprocessor`. You can repeat each option:

```sh
russet run --postprocessor PROCESSOR RECIPE
```

Replace `PROCESSOR` with a processor name from the
[processor list](/reference/processors/). The processor runs with no
arguments of its own, so it reads its input from the recipe's variables.
Processors that you name on the command line replace the ones in a recipe
list.

## Save a run report

To save a report of the run as a property list, add `--report-plist`:

```sh
russet run --report-plist REPORT_PATH RECIPE
```

Replace `REPORT_PATH` with the path for the report file. The report contains a
`failures` array, with the recipe name, identifier, and error message of each
failed recipe, and a `summary_results` dictionary with the same information as
the printed summary.

Russet also writes a receipt for each recipe in the recipe's cache folder,
under `receipts`. A receipt records the recipe's input variables and each
processor's input and output.

## Check the result

`russet run` exits with one of the following statuses:

- `0`: every recipe ran without an error.
- `70`: at least one recipe failed while it ran. The other recipes still ran.
- `1`: Russet stopped before it ran any recipe.

For every status, see [Exit codes](/reference/exit-codes/).

## Install software with a recipe

On macOS, `install` recipes install software on the computer that runs them.
To run an install recipe, use `russet install` with the software's name:

```sh
russet install NAME
```

Replace `NAME` with the name before `.install`, such as `Firefox`. Russet runs
`NAME.install`. The `russet-installd` helper service installs the software,
so you don't need to run `russet` with `sudo`.

## What's next

- [Schedule recipe runs](/guides/schedule-runs/)
- [`russet run` reference](/reference/cli/russet-run/)
- [Exit codes](/reference/exit-codes/)
