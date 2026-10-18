---
title: russet migrate
description: Copy Python AutoPkg's preferences into Russet's own preferences.
---

`russet migrate` copies the preferences that Python AutoPkg uses into
Russet's own preferences. It doesn't change Python AutoPkg's preferences.

Russet doesn't read Python AutoPkg's preferences when it runs recipes, so
this command is how you bring a Python AutoPkg setup over to Russet. If you
don't run it, Russet starts with empty preferences.

## Syntax

```sh
russet migrate [OPTIONS]
```

Replace `OPTIONS` with any of the options in the following table.

## Description

Russet reads AutoPkg's preferences and Russet's own preferences. For the
location of each on your platform, see
[Preferences](/reference/preferences/#preference-locations) and
[AutoPkg's preferences](/reference/preferences/#autopkgs-preferences). For each key in
AutoPkg's preferences, Russet does one of the following:

| Outcome | Meaning |
| --- | --- |
| `copied` | Russet's preferences don't have the key, so Russet copies AutoPkg's value. |
| `unchanged` | Russet's preferences already have the same value. |
| `conflict` | Russet's preferences have a different value, and Russet keeps its own value. |
| `replaced` | With `--force`, Russet's preferences had a different value, and Russet replaces it with AutoPkg's value. |

Russet prints one line for each key with its outcome, sorted by key,
followed by a summary of the counts. If any key conflicts, Russet says that
it kept its own value and that you can run the command with `--force` to use
AutoPkg's values.
Without `--dry-run`, if Russet saved any key, it prints the number of
preferences that it wrote and where it wrote them: the file on Linux and
Windows, or the `com.github.russet` preference domain on macOS. If AutoPkg's
preferences are empty, Russet prints `AutoPkg has no preferences to migrate.`

Russet copies every key, including your own keys that recipes use as
variables, and keeps each value's type. Supported types include strings,
numbers, Boolean values, dates, data, arrays, and dictionaries. On macOS, this includes keys in
`/Library/Preferences/com.github.autopkg.plist` and the keys that a
configuration profile sets. For details, see
[AutoPkg's preferences](/reference/preferences/#autopkgs-preferences).
Russet saves them all to the current user's preferences.

Python AutoPkg and Russet have different default folders. When AutoPkg's
`RECIPE_SEARCH_DIRS` or `RECIPE_OVERRIDE_DIRS` is unset or empty, and one of
AutoPkg's default folders for that key exists, Russet copies AutoPkg's
default value and labels the line `(AutoPkg's default)`. AutoPkg's defaults
are the following:

| Key | AutoPkg's default |
| --- | --- |
| `RECIPE_SEARCH_DIRS` | `.`, `~/Library/AutoPkg/Recipes`, and `/Library/AutoPkg/Recipes` |
| `RECIPE_OVERRIDE_DIRS` | `~/Library/AutoPkg/RecipeOverrides` |

Russet then finds the recipes and overrides that Python AutoPkg found.
Russet doesn't copy AutoPkg's defaults for `RECIPE_REPO_DIR`, `CACHE_DIR`, or
`RECIPE_MAP_PATH`, so new clones, downloads, and the recipe map go to
Russet's own folders.

`russet migrate` copies preferences only. It doesn't move or copy any
folders or files, so paths such as `RECIPE_REPOS` or `RECIPE_SEARCH_DIRS`
that point into `~/Library/AutoPkg` keep pointing there, and Russet uses those
folders in place. It doesn't copy a GitHub token file, but if
`~/.russet_gh_token` doesn't exist, Russet reads `~/.autopkg_gh_token`.
Russet never changes AutoPkg's preferences, so Python AutoPkg keeps working
as it did before.

You can run the command again safely. After a migration, every key that you
didn't change is `unchanged`.

`migrate` is a Russet-only verb. The verb list that `russet help` prints
matches Python AutoPkg's verbs, so it doesn't include `migrate`.

## Options

| Option | Description |
| --- | --- |
| `-n`, `--dry-run` | Prints what Russet would change without writing anything. |
| `-f`, `--force` | Replaces Russet's values that differ from AutoPkg's values. |
| `-h`, `--help` | Prints the help and exits with status `0`. |

`russet migrate` doesn't accept `--prefs`. It always reads AutoPkg's
preferences and writes Russet's preferences. To test the command with other
files, set the `RUSSET_AUTOPKG_PREFERENCES_FILE` and `RUSSET_PREFERENCES_FILE`
environment variables. For details, see
[Environment variables](/reference/environment-variables/#russet-variables).

## Exit status

| Status | Meaning |
| --- | --- |
| `0` | Russet finished, including when keys conflict. |
| `1` | Russet couldn't read or save preferences. |
| `2` | You passed an unexpected argument. Russet prints the usage and an error such as `russet migrate: error: unexpected argument: ARGUMENT`. |

## Examples

To see what Russet would copy without changing anything, run the following
command:

```sh
russet migrate --dry-run
```

The output looks like the following:

```text
  RECIPE_REPOS        copied
  RECIPE_SEARCH_DIRS  copied

2 copied, 0 replaced, 0 unchanged, 0 conflicting.
Dry run: nothing was written.
```

To copy AutoPkg's preferences, run the following command:

```sh
russet migrate
```

To copy AutoPkg's preferences and replace any Russet values that differ, run
the following command:

```sh
russet migrate --force
```

## Related pages

- [Preferences](/reference/preferences/)
- [Configure preferences](/guides/configure-preferences/)
- [Switch from Python AutoPkg](/guides/switch-from-autopkg/)
