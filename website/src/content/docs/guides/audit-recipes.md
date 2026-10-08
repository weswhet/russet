---
title: Audit recipes
description: Check recipes for insecure downloads, missing code signature verification, and other risks before you run them.
---

This page shows you how to audit recipes for common risks before you run
them, and how to fail an automated check when an audit finds a problem.

## Before you begin

- [Add the recipe repositories](/guides/add-recipe-repositories/) that
  contain the recipes.

## Audit a recipe

An audit reads a recipe and its parent recipes without running any
processors. To audit one or more recipes, run the following command:

```sh
russet audit RECIPE
```

Replace `RECIPE` with a recipe's short name, identifier, or path. To audit
several recipes, list them, or use `--recipe-list` with a recipe list file.

For each recipe, the output lists any findings. A recipe without findings
produces the following line:

```text
RECIPE: no audit flags triggered.
```

## Audit checks

Russet runs the following checks:

| Check | Severity | What it finds |
| --- | --- | --- |
| `insecure_protocol` | Warning | Input values and arguments that start with `http:` or `ftp:`. |
| `missing_codesig` | Warning | Recipes that use `URLDownloader` or `URLDownloaderPython` without a `CodeSignatureVerifier` step. |
| `weak_hash` | Warning | `ChocolateyPackager` steps whose `installer_checksum_type` is `md5` or `sha1`. |
| `path_safety` | Warning | Path arguments that could refer to files outside the recipe's folders, such as paths that contain `..`. |
| `sensitive_input` | Error | Input values that look like literal credentials, such as a `PASSWORD` key with a value that isn't a `%VARIABLE%` reference. Russet never prints the value. |
| `modification_processor` | Info | Processors that build packages or disk images, and file-changing processors, such as `Copier`, that run before them. |
| `non_core_processor` | Info | Processors that aren't among AutoPkg's built-in processors. This check also lists repository processors that Russet implements natively, such as `MozillaURLProvider`. |

To print the check names, run `russet audit --list-checks`.

To report only some checks, use `--only-check` with a comma-separated list of
check names. To leave some checks out, use `--skip-check`. You can't use both
options in the same command.

## Fail when an audit finds a problem

To use an audit in an automated check, add `--fail-on` with a severity:

```sh
russet audit --fail-on SEVERITY --recipe-list RECIPE_LIST
```

Replace the following:

- `SEVERITY`: `info`, `warning`, or `error`. The command exits with status
  `1` if any finding has this severity or a higher one.
- `RECIPE_LIST`: the path to a recipe list file.

## Get machine-readable results

To process the results in a script, add `--json` or `--plist`. JSON output
lists each recipe with its findings, and each finding includes its check
name, details, and severity. You can't use both options in the same command.

## What's next

- [Recipe trust](/concepts/recipe-trust/)
- [`russet audit` reference](/reference/cli/russet-audit/)
