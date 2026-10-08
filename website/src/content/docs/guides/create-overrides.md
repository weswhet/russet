---
title: Create recipe overrides
description: Create an override for a recipe, change its input values, and keep its trust information current.
---

This page shows you how to create a recipe override, change its input
values, and verify and update its trust information when the parent recipe
changes.

## Before you begin

- [Add the recipe repository](/guides/add-recipe-repositories/) that contains
  the recipe.
- Read [Recipe trust](/concepts/recipe-trust/) to learn what trust
  information protects.

## Create an override

An override is a small recipe in your overrides folder that names a parent
recipe, copies the parent's input variables so that you can change them, and
records the parent's trust information.

To create an override, run the following command:

```sh
russet make-override RECIPE
```

Replace `RECIPE` with the short name or identifier of the recipe, such as
`Firefox.munki`. Russet finds recipes in the top level of each search folder
and one folder down, so a short name works for recipes in a recipe
repository.

Russet saves the override in the first folder in `RECIPE_OVERRIDE_DIRS`,
which is `~/Library/AutoPkg/RecipeOverrides` by default. The override's
identifier starts with `local.`, such as `local.munki.Firefox`.

You can change how Russet creates the override with the following options:

- `--format yaml`: save the override in YAML instead of a property list. To
  make YAML the default, set the `RECIPE_OVERRIDE_FORMAT` preference to
  `yaml`.
- `--name FILENAME`: save the override under a different filename.
- `--force`: replace an existing override file.
- `--ignore-deprecation`: create an override for a deprecated recipe, or for
  a recipe with a deprecated parent.

## Change input values

To change what a recipe does, edit the values in the override's `Input`
dictionary. For example, a Munki recipe's override can set the catalog that
imported items join:

```xml
<key>Input</key>
<dict>
    <key>MUNKI_REPO_SUBDIR</key>
    <string>apps/firefox</string>
    <key>pkginfo</key>
    <dict>
        <key>catalogs</key>
        <array>
            <string>testing</string>
        </array>
    </dict>
</dict>
```

The keys available depend on the recipe. To see a recipe's input variables,
run `russet info RECIPE`.

Don't edit `ParentRecipeTrustInfo` by hand. Russet maintains it.

## Verify trust information

When a parent recipe or a processor it uses changes, the override's trust
information no longer matches. `russet run` refuses to run the override
until you review the change and update the trust information.

To check an override without running it, run the following command:

```sh
russet verify-trust-info -v OVERRIDE
```

Replace `OVERRIDE` with the override's short name. To check several
overrides, list them, or use `--recipe-list` with a recipe list file.

If the trust information matches, the output is the following:

```text
OVERRIDE: OK
```

If it doesn't match, the output names what changed:

```text
OVERRIDE: FAILED
    Parent recipe com.github.autopkg.download.firefox-rc-en_US contents differ from expected
```

The command exits with status `1` if any override fails.

## Review a change and update trust information

Before you trust a changed recipe, review what changed. Recipe repositories
are Git repositories, so you can use Git to see the change. To review and
accept a change, follow these steps:

1. Find the path of the parent recipe:

   ```sh
   russet info OVERRIDE
   ```

   The `Parent recipe(s)` line lists the path of each parent recipe.

1. Show the recent changes to the parent recipe's file:

   ```sh
   git -C "REPOSITORY_FOLDER" log -p -- "RECIPE_FILE"
   ```

   Replace the following:

   - `REPOSITORY_FOLDER`: the recipe repository's folder, such as
     `~/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes`.
   - `RECIPE_FILE`: the parent recipe's path inside that folder.

1. If you trust the change, update the override's trust information:

   ```sh
   russet update-trust-info OVERRIDE
   ```

   The output names the file that Russet updated:

   ```text
   Wrote updated /Users/USERNAME/Library/AutoPkg/RecipeOverrides/OVERRIDE.recipe
   ```

## Require trust information

By default, Russet runs an override that has no trust information and prints a
warning. To treat a missing trust record as an error, set the
`FAIL_RECIPES_WITHOUT_TRUST_INFO` preference to `true`. For details, see
[Preferences](/reference/preferences/).

To run recipes even when trust verification fails, add
`--ignore-parent-trust-verification-errors` to `russet run`. Use this option
only for testing. It skips the check that protects you from changes that
you haven't reviewed.

## What's next

- [Recipe trust](/concepts/recipe-trust/)
- [Run recipes](/guides/run-recipes/)
- [`russet make-override` reference](/reference/cli/russet-make-override/)
