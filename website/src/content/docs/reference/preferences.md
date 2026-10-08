---
title: Preferences
description: Where Russet reads and writes preferences on each platform, and the preference keys that it uses.
---

Russet reads the same preference keys as Python AutoPkg. This page lists where
Russet stores preferences on each platform, the file formats that it accepts,
and the keys that change its behavior.

## Preference locations

Russet reads preferences from a different place on each platform:

| Platform | Location |
| --- | --- |
| macOS | The `com.github.autopkg` preference domain for the current user |
| Linux | `config.plist` or `config.json` in `$XDG_CONFIG_HOME/Autopkg`, or in `~/.config/Autopkg` when `XDG_CONFIG_HOME` isn't set |
| Windows | `config.plist` or `config.json` in `%LOCALAPPDATA%\Autopkg` |

On Linux and Windows, Russet checks `config.plist` before `config.json` and
uses the first file that contains at least one key. If Russet can't parse an
existing file, it reports an error.

On macOS, Russet reads the keys that the current user's `com.github.autopkg`
domain contains. Russet might not read keys that exist only in another domain
level, such as `/Library/Preferences` or a configuration profile.

## Preference files

You can pass a preference file to any command with `--prefs FILE`. Russet
reads the platform location first and then applies the values from the file
on top. A preference file can be one of the following formats:

- An XML property list.
- A binary property list.
- A JSON object. JSON can't store dates or binary data.

When a command changes preferences, such as `russet repo-add`, Russet writes
the change to a file if it loaded one:

- With `--prefs FILE`, Russet writes all of the merged preferences to that
  file, including values that it read from the platform location.
- On Linux and Windows without `--prefs`, Russet writes to the `config.plist`
  or `config.json` file that it loaded. If Russet didn't load a file, the command
  fails with the error `No writable preference file loaded; use --prefs FILE`.
- On macOS without `--prefs`, Russet writes the changed keys to the
  `com.github.autopkg` domain.

Russet keeps a file's format when it saves it, except that it writes property
lists in XML format.

## Preference keys

The following keys change how Russet runs. Paths that start with `~` refer to
the current user's home folder on every platform, including Linux and
Windows.

| Key | Default | Description |
| --- | --- | --- |
| `RECIPE_SEARCH_DIRS` | `.`, `~/Library/AutoPkg/Recipes`, `/Library/AutoPkg/Recipes` | Folders to search for recipes. A string or an array of strings. |
| `RECIPE_OVERRIDE_DIRS` | `~/Library/AutoPkg/RecipeOverrides` | Folders to search for recipe overrides. A string or an array of strings. |
| `RECIPE_REPO_DIR` | `~/Library/AutoPkg/RecipeRepos` | Folder where `russet repo-add` clones recipe repositories. |
| `RECIPE_REPOS` | Empty | The recipe repositories that `russet repo-add` added, keyed by local path. Russet maintains this key. |
| `RECIPE_MAP_PATH` | `~/Library/AutoPkg/recipe_map.json` | Location of the recipe map file. |
| `DISABLE_RECIPE_MAP` | `false` | If `true`, Russet doesn't create or refresh the recipe map. |
| `CACHE_DIR` | `~/Library/AutoPkg/Cache` | Folder for downloads, build products, receipts, and run results. |
| `GIT_PATH` | `git` from `PATH` | Git executable that recipe repository commands use. |
| `CURL_PATH` | `curl` from `PATH`, then `/usr/bin/curl` | curl executable for downloads and GitHub requests. |
| `GITHUB_TOKEN` | None | GitHub token for API requests. Russet reads this token only from preferences. |
| `GITHUB_TOKEN_PATH` | `~/.autopkg_gh_token` | File that contains a GitHub token, used when `GITHUB_TOKEN` isn't set. |
| `FAIL_RECIPES_WITHOUT_TRUST_INFO` | `false` | If `true`, a recipe override without trust information is an error instead of a warning. |
| `MUNKI_REPO` | None | Path to your Munki repository. Required by the Munki import processors. |
| `MUNKI_REPO_PLUGIN` | `FileRepo` | Munki repository backend. Russet supports only `FileRepo`. |

Russet also passes every preference key to recipes as a variable. A recipe
override or the `--key` option can supply a different value for a single run.

:::caution
If `MUNKI_REPO_PLUGIN` names a backend other than `FileRepo`, or if
`MUNKI_REPO` is a URL other than a `file://` URL, Russet rejects every recipe
that you run, including recipes that don't use Munki.
:::

## Examples

To set the Munki repository path on macOS, run the following command:

```sh
defaults write com.github.autopkg MUNKI_REPO /Users/Shared/munki_repo
```

To create a preference file on Linux that sets the cache folder, run the
following commands:

```sh
mkdir -p ~/.config/Autopkg
printf '{\n  "CACHE_DIR": "/var/tmp/russet-cache"\n}\n' > ~/.config/Autopkg/config.json
```

## Related pages

- [Configure preferences](/guides/configure-preferences/)
- [Files and paths](/reference/files-and-paths/)
- [Environment variables](/reference/environment-variables/)
