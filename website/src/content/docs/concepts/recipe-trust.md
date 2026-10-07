---
title: Recipe trust
description: How recipe overrides record trust information, what Russet verifies before it runs a recipe, and where you must keep trusted overrides.
---

Recipes download and run software from the internet, and the recipes
themselves come from Git repositories that other people maintain. Trust
information lets you approve a specific version of a recipe and notice when
it changes. This page explains what trust information records and how Russet
uses it.

## What trust information records

When you create an override, Russet adds a `ParentRecipeTrustInfo` dictionary
to it. The dictionary has three parts:

- **`parent_recipes`:** for each parent recipe, its path, a hash of its
  contents in the `sha256_hash` key, and, when the recipe is in a Git
  repository, the hash of the last Git commit that changed it.
- **`non_core_processors`:** a hash of the source file of each processor that
  a recipe repository supplies, for the processors that Russet implements
  natively, such as `MakeCatalogsProcessor`.
- **`scripts`:** a hash of each Git-tracked script that a `PkgCreator` step
  includes in a package.

Russet records the source files of repository processors even though it runs
its own implementation of them. That keeps trust information compatible with
Python AutoPkg, and it still alerts you when a recipe repository changes the
processor.

## What Russet verifies

Before `autopkg run` runs an override, Russet computes the same hashes again
and compares them with the trust information. Verification fails when any of
the following is true:

- A parent recipe's contents changed.
- A processor source file or a trusted script changed or is missing.
- The override changed while Russet was loading it.

A new script that isn't in the trust information produces a warning, not a
failure.

If verification fails, Russet stops before it runs any recipe in the command
and exits with status `1`. To review and accept a change, see
[Create recipe overrides](/guides/create-overrides/#review-a-change-and-update-trust-information).

## Where trusted overrides must be

Russet honors trust information only in an override that meets both of the
following conditions:

- The override is inside one of your override folders.
- The override isn't inside a recipe repository. Recipe repositories are the
  folder in `RECIPE_REPO_DIR`, where `autopkg repo-add` clones repositories,
  and every repository in `RECIPE_REPOS`.

The second condition stops a recipe repository from supplying its own trust
information. If an override is inside a recipe repository, for example
because an override folder points into a cloned repository, Russet reports
the following error:

```text
Trust records must come from a local override outside recipe repositories
```

Python AutoPkg doesn't have this check.

## Recipes without trust information

A recipe that isn't an override has no trust information. By default, Russet
runs it and prints a warning. To make a missing trust record an error, set
the `FAIL_RECIPES_WITHOUT_TRUST_INFO` preference to `true`.

## What's next

- [Create recipe overrides](/guides/create-overrides/)
- [Compatibility with AutoPkg](/concepts/compatibility/)
- [Troubleshooting](/resources/troubleshooting/)
