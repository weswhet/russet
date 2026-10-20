---
title: Preferences
description: Where Russet reads and saves preferences on each platform, and the preference keys that it uses.
---

Russet reads the same preference keys as Python AutoPkg. This page lists where
Russet reads and saves preferences on each platform, the file formats that it
accepts, and the keys that change its behavior.

## Preference locations

Russet reads its own preferences, which are separate from Python AutoPkg's
preferences. Russet reads them from a different place on each platform:

| Platform | Location |
| --- | --- |
| macOS | The `com.github.russet` preference domain for the current user |
| Linux | `config.plist` or `config.json` in `$XDG_CONFIG_HOME/Russet`, or in `~/.config/Russet` when `XDG_CONFIG_HOME` isn't set |
| Windows | `config.plist` or `config.json` in `%LOCALAPPDATA%\Russet` |

On Linux and Windows, Russet checks `config.plist` before `config.json` and
uses the first file that contains at least one key. If Russet can't parse an
existing file, it reports an error. If the folder has neither file, Russet
creates the folder and `config.json` the first time that it saves a change.

On macOS, Russet reads the same keys that Python AutoPkg reads from its
domain:

- Keys in the current user's `com.github.russet` domain.
- Keys in `/Library/Preferences/com.github.russet.plist`, which apply to all
  users. A key in the current user's domain takes precedence.
- Keys that a configuration profile sets, when Russet reads the key by name.
  Russet reads `CACHE_DIR`, `CURL_PATH`, `DISABLE_RECIPE_MAP`,
  `FAIL_RECIPES_WITHOUT_TRUST_INFO`, `GIT_PATH`, `GITHUB_TOKEN`, `MUNKI_REPO`,
  `MUNKI_REPO_PLUGIN`, `RECIPE_MAP_PATH`, `RECIPE_OVERRIDE_DIRS`,
  `RECIPE_OVERRIDE_FORMAT`, `RECIPE_REPO_DIR`, `RECIPE_REPOS`,
  `RECIPE_SEARCH_DIRS`, `RussetJobs`, and `UseRussetDownloader` by name. A
  value from a configuration profile takes precedence over the other levels.

Russet saves changes to the current user's domain only.

## AutoPkg's preferences

Russet doesn't read Python AutoPkg's preferences when it runs recipes, and it
never changes them. Python AutoPkg's preferences are in the
`com.github.autopkg` domain on macOS, in `~/.config/Autopkg` or
`$XDG_CONFIG_HOME/Autopkg` on Linux, and in `%LOCALAPPDATA%\Autopkg` on
Windows.

Russet reads AutoPkg's preferences the way that Python AutoPkg does. On
macOS, it reads the same domain levels as it does for its own preferences. On
Linux and Windows, it uses `config.plist` and then `config.json`, and skips a
file that's empty or that it can't parse, without reporting an error.

Only the following commands read AutoPkg's preferences:

- [`russet migrate`](/reference/cli/russet-migrate/) copies them into
  Russet's preferences. If you used Python AutoPkg, run this command to bring
  your settings over. Until you do, Russet starts with empty preferences.
- [`russet repo-delete`](/reference/cli/russet-repo-delete/) keeps the folder
  of a recipe repository that AutoPkg's preferences list, so that Python
  AutoPkg keeps working.

## Preference files

You can pass a preference file to most commands with `--prefs FILE`. Russet
reads its own preferences first and then applies the values from the file.
When a key appears in both, the value from the file wins. A preference file
can be one of the following formats:

- An XML property list.
- A binary property list.
- A JSON object. JSON can't store dates or binary data.

A recipe override, an environment variable, or the `--key` option can supply
a different value for a single run. For details, see
[Environment variables](/reference/environment-variables/#recipe-variables).

## Where Russet saves changes

When a command changes preferences, such as `russet repo-add` or
`russet repo-delete`, Russet saves the changes as follows:

- With `--prefs FILE`, Russet writes to that file. The file keeps its own
  values and gets the changed keys. Russet doesn't copy other values from
  Russet's preferences into the file.
- Without `--prefs`, Russet writes the changed keys to Russet's preferences:
  the `com.github.russet` domain on macOS, or Russet's `config.plist` or
  `config.json` file on Linux and Windows.

Russet never writes to AutoPkg's preferences.

Russet keeps a file's format when it saves it, except that it writes property
lists in XML format.

To test Russet without reading or changing Russet's real preferences, set
the `RUSSET_PREFERENCES_FILE` environment variable to the path of a file. For
details, see
[Environment variables](/reference/environment-variables/#russet-variables).

## Preference keys

The following keys change how Russet runs. Paths that start with `~` refer to
the current user's home folder on every platform, including Linux and
Windows. Russet uses a default only when your preferences don't set the key.

| Key | Default | Description |
| --- | --- | --- |
| `RECIPE_SEARCH_DIRS` | `.`, `~/Library/Russet/Recipes`, `/Library/Russet/Recipes` | Folders to search for recipes. A string or an array of strings. |
| `RECIPE_OVERRIDE_DIRS` | `~/Library/Russet/RecipeOverrides` | Folders to search for recipe overrides. A string or an array of strings. `russet make-override` saves new overrides in the first folder. |
| `RECIPE_REPO_DIR` | `~/Library/Russet/RecipeRepos` | Folder where `russet repo-add` clones recipe repositories. |
| `RECIPE_REPOS` | Empty | The recipe repositories that `russet repo-add` added, keyed by local path. Russet maintains this key. |
| `RECIPE_MAP_PATH` | `~/Library/Russet/recipe_map.json` | Location of the recipe map file. |
| `DISABLE_RECIPE_MAP` | `false` | If `true`, Russet doesn't create or refresh the recipe map. |
| `CACHE_DIR` | `~/Library/Russet/Cache` | Folder for downloads, build products, receipts, and run results. |
| `GIT_PATH` | `git` from `PATH` | Git executable that recipe repository commands use. |
| `CURL_PATH` | `curl` from `PATH`, then `/usr/bin/curl` | curl executable for downloads and GitHub requests. If you set this key, `URLDownloader` and `URLDownloaderPython` use this executable for every download instead of Russet's downloader. |
| `RussetJobs` | `1` | Number of recipes that `russet run` and `russet install` run at the same time. `0` runs one recipe for each CPU. The `--jobs` option overrides this preference. For details, see [Run recipes at the same time](/reference/cli/russet-run/#run-recipes-at-the-same-time). |
| `UseRussetDownloader` | `true` | If `false`, `URLDownloader` and `URLDownloaderPython` use curl for every download instead of Russet's downloader. For details, see [Downloads](#downloads). |
| `GITHUB_TOKEN` | None | GitHub token for API requests. Russet reads this token only from preferences. |
| `GITHUB_TOKEN_PATH` | `~/.russet_gh_token` | File that contains a GitHub token, used when `GITHUB_TOKEN` isn't set. When this key isn't set, or when it's set to Python AutoPkg's default, `~/.autopkg_gh_token`, Russet reads `~/.russet_gh_token`. If only `~/.autopkg_gh_token` exists, Russet reads that file instead. |
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

## Downloads

By default, `URLDownloader` and `URLDownloaderPython` download files with
Russet's built-in HTTP downloader. The downloader reuses connections between
downloads. For a file of 64 MiB or more, it can download several parts of the
file at once when the server supports it. Downloads keep the caching, file
names, and recipe variables that curl downloads produce.

Russet uses curl for a download when the downloader can't reproduce curl's
behavior exactly. Russet makes this choice before it connects to the server.
The download uses curl in the following cases:

- The recipe's `curl_opts` contains an option other than `--location`,
  `--fail`, `--silent`, `--show-error`, `--header`, `--user-agent`, or
  `--referer`, or their short forms.
- The URL doesn't use HTTP or HTTPS, or it contains a user name or password.
- You set `CURL_PATH`.
- You set a proxy environment variable, such as `https_proxy`.
- A curl configuration file, such as `~/.curlrc`, exists.

The downloader sends the same `User-Agent` header as the curl that Russet
would otherwise run, such as `curl/8.7.1`, unless the recipe supplies its own.
It trusts the same certificates as curl. On macOS, these are the bundled
certificates or the file that `SSL_CERT_FILE` names. On Linux and Windows,
these are the operating system's certificates.

To use curl for every download, set `UseRussetDownloader` to `false`. The
value can be a Boolean or a string such as `false`, `no`, or `0`. You can also
set it for one run with the `AUTOPKG_UseRussetDownloader` environment
variable or with `--key UseRussetDownloader=false`.

To make a recipe's download fail instead of using curl, use the
[`RussetURLDownloader`](/reference/processors/#russet-processors) processor
instead of `URLDownloader`.

## Examples

To use curl instead of Russet's downloader on macOS, run the following
command:

```sh
defaults write com.github.russet UseRussetDownloader -bool false
```

To run four recipes at a time on macOS, run the following command:

```sh
defaults write com.github.russet RussetJobs -int 4
```

To set the Munki repository path on macOS, run the following command:

```sh
defaults write com.github.russet MUNKI_REPO /Users/Shared/munki_repo
```

To create a preference file on Linux that sets the cache folder, run the
following commands:

```sh
mkdir -p ~/.config/Russet
printf '{\n  "CACHE_DIR": "/var/tmp/russet-cache"\n}\n' > ~/.config/Russet/config.json
```

## Related pages

- [Configure preferences](/guides/configure-preferences/)
- [Files and paths](/reference/files-and-paths/)
- [Environment variables](/reference/environment-variables/)
