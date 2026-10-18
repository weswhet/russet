---
title: Configure preferences
description: Set Russet's preferences on macOS, Linux, and Windows, and use a separate preference file for a run.
---

This page shows you how to set Russet's preferences on each platform and how
to use a separate preference file. For the list of keys, see
[Preferences](/reference/preferences/).

## Before you begin

- [Install Russet](/get-started/install/).

## Bring over your Python AutoPkg preferences

Russet keeps its own preferences and doesn't read Python AutoPkg's
preferences when it runs recipes. If you used Python AutoPkg, copy your
settings into Russet's preferences with
[`russet migrate`](/reference/cli/russet-migrate/). Until you do, Russet
starts with empty preferences. Russet never changes Python AutoPkg's
preferences, so Python AutoPkg keeps working.

For the steps, see
[Switch from Python AutoPkg](/guides/switch-from-autopkg/#copy-your-preferences-to-russet).

## Set preferences on macOS

On macOS, Russet reads the `com.github.russet` preference domain.

To set a preference, run the following command:

```sh
defaults write com.github.russet KEY VALUE
```

Replace the following:

- `KEY`: the preference key, such as `MUNKI_REPO`.
- `VALUE`: the value to set, such as `/Users/Shared/munki_repo`.

To set a list, such as `RECIPE_OVERRIDE_DIRS`, use `-array`:

```sh
defaults write com.github.russet RECIPE_OVERRIDE_DIRS -array ~/Library/Russet/RecipeOverrides FOLDER_PATH
```

Replace `FOLDER_PATH` with the path of another folder.

To see your current preferences as Russet reads them, run `russet info`
without a recipe name.

## Set preferences on Linux

On Linux, Russet reads `config.plist` or `config.json` in
`~/.config/Russet`, or in `$XDG_CONFIG_HOME/Russet` if you set
`XDG_CONFIG_HOME`. Russet saves preference changes, such as the repositories
that `russet repo-add` adds, to that folder. If the folder has no preference
file, Russet creates the folder and `config.json` the first time that it saves
a change.

To create a preference file yourself, follow these steps:

1. Create the folder:

   ```sh
   mkdir -p ~/.config/Russet
   ```

1. Create `config.json` with at least one key. For example, the following
   command sets the cache folder:

   ```sh
   printf '{\n  "CACHE_DIR": "/var/tmp/russet-cache"\n}\n' > ~/.config/Russet/config.json
   ```

1. Confirm that Russet reads the file:

   ```sh
   russet info
   ```

   The output starts with `Current preferences:` and lists the keys that
   Russet read.

To change a preference, edit the file in a text editor.

## Set preferences on Windows

On Windows, Russet reads `config.plist` or `config.json` in
`%LOCALAPPDATA%\Russet`. As on Linux, Russet creates the folder and
`config.json` the first time that it saves a change.

To create a preference file yourself, run the following commands in
PowerShell:

```powershell
New-Item -ItemType Directory -Force "$env:LOCALAPPDATA\Russet"
Set-Content -Path "$env:LOCALAPPDATA\Russet\config.json" -Value '{ "CACHE_DIR": "C:\\RussetCache" }'
```

To change a preference, edit the file in a text editor.

## Use a separate preference file

To use a different set of preferences for one command, such as a test
configuration, pass a preference file with `--prefs`:

```sh
russet run --prefs PREFERENCE_FILE RECIPE
```

Replace the following:

- `PREFERENCE_FILE`: the path to a property list or JSON file.
- `RECIPE`: the recipe to run.

Russet reads your normal preferences first and then applies the values in the
file. When a key appears in both, the value in the file wins. When a command
changes preferences, Russet saves the changed keys to the file that you
passed, not to your normal preferences. Russet doesn't copy your normal
preferences into the file.

## What's next

- [`russet migrate`](/reference/cli/russet-migrate/)
- [Preferences](/reference/preferences/)
- [Environment variables](/reference/environment-variables/)
- [Files and paths](/reference/files-and-paths/)
