---
title: Files and paths
description: Where Russet installs its files and where it keeps preferences, recipes, caches, and rollback history.
---

This page lists the files that the Russet installers create and the folders
that Russet uses while it runs.

## Installed files

The installers put the `russet` command and its supporting files in the
following locations:

| Item | macOS and Linux | Windows |
| --- | --- | --- |
| Installation folder | `/opt/russet` | The folder that you pass with `-Destination` |
| `russet` command | `/opt/russet/russet` | `DESTINATION\russet.exe` |
| Command link | `/usr/local/bin/russet` | None. Add the folder to `PATH` yourself. |
| Installer copy | `/opt/russet/install.sh` | `DESTINATION\install.ps1` |
| Rollback history | `/opt/russet-rollbacks` | `DESTINATION-rollbacks` |

On macOS, the installer also adds the privileged helper services. Each one
is the `russet` command, run by launchd with a flag that selects the service:
`russet --server` for the package builder and `russet --installd` for the
package installer.

| Item | Path |
| --- | --- |
| Package builder launchd job | `/Library/LaunchDaemons/com.github.weswhet.russet.server.plist` |
| Package installer launchd job | `/Library/LaunchDaemons/com.github.weswhet.russet.installd.plist` |
| Package builder socket | `/var/run/russet-server` |
| Package installer socket | `/var/run/russet-installd` |
| Package builder log | `/private/var/log/russet-server` |
| Package installer log | `/private/var/log/russet-installd` |

None of these paths overlap with Python AutoPkg's installation, so Russet and
Python AutoPkg can run on the same Mac. Both read the same preferences and use
the same working folders.

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
