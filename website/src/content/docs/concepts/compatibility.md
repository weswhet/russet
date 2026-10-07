---
title: Compatibility with AutoPkg
description: What Russet keeps from AutoPkg, what it intentionally doesn't support, and how the project tests its compatibility.
---

Russet targets compatibility with AutoPkg 3.0.0, specifically AutoPkg commit
`c36e58f`, and with Munki <!-- vale Google.DateFormat = NO -->7.2.0.5787<!-- vale Google.DateFormat = YES --> for the metadata that it generates. This
page explains what that compatibility covers, what Russet intentionally
doesn't support, and how the project tests it.

## What stays the same

Russet keeps the parts of AutoPkg that your recipes, scripts, and schedules
depend on:

- **The command:** the installed command is `autopkg`, with the same verbs and
  options.
- **Recipes:** Russet reads property list recipes, with the `.recipe` or
  `.recipe.plist` extension, and YAML recipes, with the `.recipe.yaml`
  extension. Recipes keep the same
  `Identifier`, `ParentRecipe`, `MinimumVersion`, `Input`, and `Process`
  keys.
- **Overrides and trust information:** recipe overrides and their
  `ParentRecipeTrustInfo` keep the same format and verification rules.
- **Processors:** built-in processors keep their names, input variables,
  output variables, and default values.
- **Preferences:** Russet reads the same preference keys from the same
  locations.
- **Results:** receipts, report property lists, and the run results file keep
  their formats.
- **Installation layout:** on macOS, the command, helper services, launchd
  jobs, and sockets use Python AutoPkg's paths and names.

## What Russet doesn't support

Russet intentionally doesn't run Python, so it doesn't support features that
need a Python interpreter:

- **Custom processors:** a recipe repository can supply its own processors as
  Python files. Russet doesn't run them. If a recipe uses a processor that
  Russet doesn't implement, Russet reports
  `Custom or unknown processor 'NAME' is not supported` and runs none of the
  recipes that you requested.
- **Munki repository plug-ins:** Russet supports only Munki's `FileRepo`
  backend. It rejects other values of `MUNKI_REPO_PLUGIN` and Munki
  repository URLs other than `file://` URLs.
- **Python Munki libraries:** Russet rejects `force_munki_repo_lib` and
  generates Munki metadata natively.

Russet implements some processors that recipe repositories commonly supply,
such as `MozillaURLProvider` and `MakeCatalogsProcessor`. When a recipe uses
one of those names, Russet runs its native implementation and never runs the
repository's Python file. For the list, see
[Processors](/reference/processors/).

## Intentional differences

A few behaviors differ from Python AutoPkg by design:

- **Failure details:** the `traceback` field in a failure report contains a
  native backtrace instead of Python stack frames.
- **Munki options:** Russet rejects unknown `makepkginfo` options and options
  that only change `makepkginfo` output, such as `--version`, with an
  explicit error.
- **Predicates:** on Linux and Windows, `StopProcessingIf` supports a subset
  of the predicate syntax. For details, see
  [Platform support](/concepts/platform-support/).
- **Platform-specific operations:** an operation that needs another
  platform's tools fails with an error instead of running a partial
  substitute.
- **Trust location:** Russet doesn't trust an override inside a recipe
  repository. For details, see
  [Recipe trust](/concepts/recipe-trust/#where-trusted-overrides-must-be).

## How the project tests compatibility

The Russet repository contains no Python. The tests that compare Russet with
Python AutoPkg live in a separate repository,
[russet-compat](https://github.com/weswhet/russet-compat). Its suites
download Python AutoPkg at the target commit, build Russet from source, give
both the same input, and compare their results, including final variables,
failures, reports, generated metadata, file changes, and command arguments.
They run each week, and you can also run them against any Russet branch or
commit.

Some tests in the Russet repository compare Russet's results with output
that the project saved from Python AutoPkg. These tests cover areas such as
predicates, Munki catalogs, YAML recipes, and run receipts. Other jobs package
Russet on each platform and test installation, upgrades, rollback, and the
helper services.

The project also ran all 228 recipes in the `autopkg/recipes` repository on
GitHub-hosted macOS runners on October 7, 2026. The results were as follows:

- 121 recipes completed.
- 94 recipes used custom processors, so Russet rejected them before they
  ran.
- 13 recipes failed. Most failed in Python AutoPkg too, because of retired
  downloads, invalid upstream disk images, or recipes that need more input.

Russet then added native versions of the processors that those 94 recipes
use. A second run of the 94 recipes on the same day had no processor
rejections: 81 exited successfully, 12 failed, and one timed out. For the
records of both runs, see the `evidence` folder in the
[russet-compat](https://github.com/weswhet/russet-compat) repository.

Tests show compatibility for the cases that they cover. They don't prove
identical behavior for every possible recipe, so check your own recipes
before you rely on Russet in production.

## What's next

- [Switch from Python AutoPkg](/guides/switch-from-autopkg/)
- [Processors](/reference/processors/)
- [Recipe trust](/concepts/recipe-trust/)
