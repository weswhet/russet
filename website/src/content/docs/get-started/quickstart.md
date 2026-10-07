---
title: "Quickstart: run your first recipe"
description: Add the AutoPkg recipe repository, create a recipe override, run it, and verify its trust information.
---

In this quickstart, you add the AutoPkg recipe repository, create an override
for a download recipe, run the override, and verify its trust information.
The steps use macOS, because the example recipe verifies a macOS code
signature.

## Before you begin

- [Install Russet](/get-started/install/), and confirm that
  `autopkg version` prints `3.0.0`.
- Make sure that you have Git. To check, run `git --version`. If macOS
  asks you to install the command-line developer tools, install them.
- Make sure that the computer has internet access.

## Add a recipe repository

A recipe repository is a Git repository of recipes. The `autopkg/recipes`
repository on GitHub contains the recipes that the AutoPkg project maintains.

To add it, run the following command:

```sh
autopkg repo-add recipes
```

Russet clones the repository and adds it to your recipe search folders:

```text
Attempting git clone for https://github.com/autopkg/recipes...

Adding /Users/USERNAME/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes to RECIPE_SEARCH_DIRS...
Updated search path:
  '.'
  '~/Library/AutoPkg/Recipes'
  '/Library/AutoPkg/Recipes'
  '/Users/USERNAME/Library/AutoPkg/RecipeRepos/com.github.autopkg.recipes'
```

In the output, `USERNAME` is your user name.

## Find a recipe

To list the recipes for The Unarchiver, an archive utility, run the following
command:

```sh
autopkg list-recipes | grep -i unarchiver
```

The output lists three recipes:

```text
TheUnarchiver.download
TheUnarchiver.munki
TheUnarchiver.pkg
```

The `download` recipe downloads the app and verifies its code signature. To
see its details, run the following command:

```sh
autopkg info TheUnarchiver.download
```

The output includes the recipe's identifier and input variables:

```text
Description:         Download recipe for The Unarchiver. Finds and downloads the latest 'The Unarchiver' release.
Identifier:          com.github.autopkg.download.TheUnarchiver
Munki import recipe: False
Has check phase:     True
Builds package:      False
```

## Create an override

An override is your local copy of a recipe's input variables. It also records
trust information: a hash of each parent recipe, so that Russet can detect
when a recipe changes. For details, see [Recipe trust](/concepts/recipe-trust/).

To create an override, run the following command:

```sh
autopkg make-override TheUnarchiver.download
```

Russet saves the override in your overrides folder:

```text
Override file saved to /Users/USERNAME/Library/AutoPkg/RecipeOverrides/TheUnarchiver.download.recipe
```

## Run the recipe

To run the override with verbose output, run the following command:

```sh
autopkg run -v TheUnarchiver.download
```

Russet runs each processor in the recipe and prints what it does. The output
includes lines like the following:

```text
Processing TheUnarchiver.download...
SparkleUpdateInfoProvider
SparkleUpdateInfoProvider: Version retrieved from appcast: 147
URLDownloader
URLDownloader: Downloaded /Users/USERNAME/Library/AutoPkg/Cache/local.download.TheUnarchiver/downloads/TheUnarchiver-147.zip
EndOfCheckPhase
Unarchiver
CodeSignatureVerifier
CodeSignatureVerifier: Signature is valid

The following new items were downloaded:
    Download Path
    -------------
    /Users/USERNAME/Library/AutoPkg/Cache/local.download.TheUnarchiver/downloads/TheUnarchiver-147.zip
```

The version number depends on the current release of The Unarchiver.

Run the same command again. This time, the server reports that the file
hasn't changed, so Russet doesn't download it again:

```text
Processing TheUnarchiver.download...

Nothing downloaded, packaged or imported.
```

## Verify trust information

To confirm that the parent recipe hasn't changed since you created the
override, run the following command:

```sh
autopkg verify-trust-info TheUnarchiver.download
```

The output is the following:

```text
TheUnarchiver.download: OK
```

## Clean up

If you don't want to keep the files from this quickstart, delete them:

1. Delete the override:

   ```sh
   rm ~/Library/AutoPkg/RecipeOverrides/TheUnarchiver.download.recipe
   ```

1. Delete the recipe's cache folder:

   ```sh
   rm -r ~/Library/AutoPkg/Cache/local.download.TheUnarchiver
   ```

1. Optional: if you don't plan to use the recipe repository, remove it:

   ```sh
   autopkg repo-delete recipes
   ```

## What's next

- [Run recipes](/guides/run-recipes/)
- [Create recipe overrides](/guides/create-overrides/)
- [Import software into Munki](/guides/import-into-munki/)
- [How Russet works](/concepts/how-russet-works/)
