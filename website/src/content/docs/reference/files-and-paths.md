---
title: Files and paths
description: Where Russet installs its files and where it keeps preferences, recipes, caches, and rollback history.
---

This page lists the files that the Russet installers create and the folders
that Russet uses while it runs.

## Installed files

The installers put the `autopkg` command and its supporting files in the
following locations:

| Item | macOS | Linux | Windows |
| --- | --- | --- | --- |
| Installation folder | `/Library/AutoPkg` | `/usr/local/lib/autopkg` | The folder that you pass with `-Destination` |
| `autopkg` command | `/Library/AutoPkg/autopkg` | `/usr/local/lib/autopkg/autopkg` | `DESTINATION\autopkg.exe` |
| Command link | `/usr/local/bin/autopkg` | `/usr/local/bin/autopkg` | None. Add the folder to `PATH` yourself. |
| Installer copy | `/Library/AutoPkg/install.sh` | `/usr/local/lib/autopkg/install.sh` | `DESTINATION\install.ps1` |
| Rollback history | `/Library/AutoPkg-Rollbacks` | `/usr/local/lib/autopkg-rollbacks` | `DESTINATION-rollbacks` |

On macOS, the installer also adds the privileged helper services:

| Item | Path |
| --- | --- |
| Package builder | `/Library/AutoPkg/autopkgserver/autopkgserver` |
| Package installer | `/Library/AutoPkg/autopkgserver/autopkginstalld` |
| Package builder launchd job | `/Library/LaunchDaemons/com.github.autopkg.autopkgserver.plist` |
| Package installer launchd job | `/Library/LaunchDaemons/com.github.autopkg.autopkginstalld.plist` |
| Package builder socket | `/var/run/autopkgserver` |
| Package installer socket | `/var/run/autopkginstalld` |
| Package builder log | `/private/var/log/autopkgserver` |
| Package installer log | `/private/var/log/autopkginstalld` |

These paths and service names match Python AutoPkg's, so a Russet
installation replaces Python AutoPkg in place.

## Working folders

Russet uses the same default folders on every platform. A path that starts
with `~` refers to the current user's home folder, so on Linux and Windows
these folders are also under `Library/AutoPkg` in your home folder.

| Item | Default | Preference key |
| --- | --- | --- |
| Cache | `~/Library/AutoPkg/Cache` | `CACHE_DIR` |
| Recipe repositories | `~/Library/AutoPkg/RecipeRepos` | `RECIPE_REPO_DIR` |
| Recipe overrides | `~/Library/AutoPkg/RecipeOverrides` | `RECIPE_OVERRIDE_DIRS` |
| Recipe search folders | `.`, `~/Library/AutoPkg/Recipes`, `/Library/AutoPkg/Recipes` | `RECIPE_SEARCH_DIRS` |
| Recipe map | `~/Library/AutoPkg/recipe_map.json` | `RECIPE_MAP_PATH` |
| GitHub token file | `~/.autopkg_gh_token` | `GITHUB_TOKEN_PATH` |

## Cache contents

Russet organizes the cache as follows:

| Path | Contents |
| --- | --- |
| `CACHE_DIR/RECIPE_IDENTIFIER` | The recipe's cache folder, available to recipes as `RECIPE_CACHE_DIR`. |
| `CACHE_DIR/RECIPE_IDENTIFIER/downloads` | Downloaded files and their `.info.json` metadata files. |
| `CACHE_DIR/RECIPE_IDENTIFIER/receipts` | A receipt for each run, named `RECIPE_NAME-receipt-YYYYMMDD-HHMMSS.plist`. |
| `CACHE_DIR/autopkg_results.plist` | Results of the most recent run. |

## Preference files

For preference locations on each platform, see
[Preferences](/reference/preferences/).

## Related pages

- [Install Russet](/get-started/install/)
- [Preferences](/reference/preferences/)
- [How Russet works](/concepts/how-russet-works/)
