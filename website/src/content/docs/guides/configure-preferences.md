---
title: Configure preferences
description: Set Russet's preferences on macOS, Linux, and Windows, and use a separate preference file for a run.
---

This page shows you how to set Russet's preferences on each platform and how
to use a separate preference file. For the list of keys, see
[Preferences](/reference/preferences/).

## Before you begin

- [Install Russet](/get-started/install/).

## Set preferences on macOS

On macOS, Russet reads the `com.github.autopkg` preference domain, the same
domain that Python AutoPkg uses. If you used Python AutoPkg, Russet already
has your preferences.

To set a preference, run the following command:

```sh
defaults write com.github.autopkg KEY VALUE
```

Replace the following:

- `KEY`: the preference key, such as `MUNKI_REPO`.
- `VALUE`: the value to set, such as `/Users/Shared/munki_repo`.

To set a list, such as `RECIPE_OVERRIDE_DIRS`, use `-array`:

```sh
defaults write com.github.autopkg RECIPE_OVERRIDE_DIRS -array ~/Library/AutoPkg/RecipeOverrides FOLDER_PATH
```

Replace `FOLDER_PATH` with the path of another folder.

To see your current preferences as Russet reads them, run `autopkg info`
without a recipe name.

## Set preferences on Linux

On Linux, Russet reads `config.plist` or `config.json` in
`~/.config/Autopkg`, or in `$XDG_CONFIG_HOME/Autopkg` if you set
`XDG_CONFIG_HOME`. Russet saves preference changes, such as the repositories
that `autopkg repo-add` adds, only to a file that it loaded, and it ignores a
file with no keys.

To create a preference file, follow these steps:

1. Create the folder:

   ```sh
   mkdir -p ~/.config/Autopkg
   ```

1. Create `config.json` with at least one key. For example, the following
   command sets an empty `RECIPE_REPOS` dictionary, which `autopkg repo-add`
   fills in later:

   ```sh
   printf '{\n  "RECIPE_REPOS": {}\n}\n' > ~/.config/Autopkg/config.json
   ```

1. Confirm that Russet reads the file:

   ```sh
   autopkg info
   ```

   The output starts with `Current preferences:` and lists the keys in the
   file.

To change a preference, edit the file in a text editor.

## Set preferences on Windows

On Windows, Russet reads `config.plist` or `config.json` in
`%LOCALAPPDATA%\Autopkg`. As on Linux, the file must contain at least one key
before Russet can save changes to it.

To create a preference file, run the following commands in PowerShell:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Autopkg"
Set-Content -Path "$env:LOCALAPPDATA\Autopkg\config.json" -Value '{ "RECIPE_REPOS": {} }'
```

To change a preference, edit the file in a text editor.

## Use a separate preference file

To use a different set of preferences for one command, such as a test
configuration, pass a preference file with `--prefs`:

```sh
autopkg run --prefs PREFERENCE_FILE RECIPE
```

Replace the following:

- `PREFERENCE_FILE`: the path to a property list or JSON file.
- `RECIPE`: the recipe to run.

Russet reads your normal preferences first and then applies the values in the
file. When a command changes preferences, Russet saves all of the merged
values to the file that you passed, not to your normal preferences.

## What's next

- [Preferences](/reference/preferences/)
- [Environment variables](/reference/environment-variables/)
- [Files and paths](/reference/files-and-paths/)
