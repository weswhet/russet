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
| `CURL_PATH` | `curl` from `PATH`, then `/usr/bin/curl` | curl executable for downloads and GitHub requests. If you set this key, `URLDownloader` and `URLDownloaderPython` use this executable for every download instead of Russet's downloader. |
| `RussetJobs` | `1` | Number of recipes that `russet run` and `russet install` run at the same time. `0` runs one recipe for each CPU. The `--jobs` option overrides this preference. For details, see [Run recipes at the same time](/reference/cli/russet-run/#run-recipes-at-the-same-time). |
| `UseRussetDownloader` | `true` | If `false`, `URLDownloader` and `URLDownloaderPython` use curl for every download instead of Russet's downloader. For details, see [Downloads](#downloads). |
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

## Examples

To use curl instead of Russet's downloader on macOS, run the following
command:

```sh
defaults write com.github.autopkg UseRussetDownloader -bool false
```

To run four recipes at a time on macOS, run the following command:

```sh
defaults write com.github.autopkg RussetJobs -int 4
```

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
