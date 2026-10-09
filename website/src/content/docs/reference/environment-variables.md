---
title: Environment variables
description: The environment variables that change how Russet runs recipes and finds its files.
---

This page lists the environment variables that Russet reads. For settings that
persist between runs, use [preferences](/reference/preferences/) instead.

## Recipe variables

When you run `russet run` or `russet install`, Russet turns every
environment variable whose name starts with `AUTOPKG_` into a recipe
variable. The variable name is the part after the prefix. For example,
`AUTOPKG_MUNKI_REPO` sets the recipe variable `MUNKI_REPO`.

An environment variable overrides the recipe's `Input` values and your
preferences. Values from a recipe list and from the `--key` option override
the environment variable. These variables can't supply `GITHUB_TOKEN`, which
Russet reads only from preferences.

To see the variables that Russet applies, run the recipe with `-vv`.

## Russet variables

The following variables change Russet's own behavior:

| Variable | Description |
| --- | --- |
| `AUTOPKG_RECIPE_MAP_PATH` | Location of the recipe map file. Russet ignores this variable when it runs as root and prints a security warning. |
| `AUTOPKG_DISABLE_RECIPE_MAP` | If set to any non-empty value, Russet doesn't create or refresh the recipe map. |
| `RUSSET_NATIVE` | On macOS, uses Russet's built-in replacements instead of Apple's tools, for comparing the two. Set it to `all` or to a comma-separated list of tool names. Today the replacements are `aa` (extraction), `codesign` (verification), `ditto`, `hdiutil` (reading and creating images), `icons` (Munki icon extraction), `installer` (read-only queries), `mkbom`, `pkgbuild`, `pkgutil` (expanding, flattening, and checking package signatures), and `xar`. Russet stops with an error if the value names an unknown tool or one without a replacement. Linux always uses the replacements, and recipes and preferences can't set this variable. |
| `RUSSET_PKG_OWNER` | Owner, as `uid:gid`, that packages built on Linux record for files owned by the user running Russet. Defaults to `501:20`, the first user and the `staff` group on a Mac, which is what the macOS helper records. `chown` entries in the request still apply. |
| `RUSSET_SCRATCH_DIR` | Folder for disk images that Russet reads with its built-in reader, on Linux or with `RUSSET_NATIVE`. Defaults to the system temporary folder. Each image needs free space about the size of its contents. |

## System variables

Russet also reads the following standard variables:

| Variable | Platform | Description |
| --- | --- | --- |
| `HOME` | All | Home folder for paths that start with `~`. On Windows, Russet uses `HOME` when it's set and `USERPROFILE` otherwise. |
| `USERPROFILE` | Windows | Home folder when `HOME` isn't set. |
| `XDG_CONFIG_HOME` | Linux | Parent of the `Autopkg` preference folder. Defaults to `~/.config`. |
| `LOCALAPPDATA` | Windows | Parent of the `Autopkg` preference folder. Russet fails if this variable isn't set. |
| `PATH` | All | Search path for curl, Git, and other tools that Russet runs by name. |
| `SSL_CERT_FILE` | All | Certificate bundle for Transport Layer Security (TLS) connections. On macOS, if this variable names an existing file, Russet doesn't supply its bundled certificates to curl or to its downloader. |
| `SSL_CERT_DIR` | Linux, macOS | Folder of hashed certificates that `URLDownloaderPython` adds to its trust store. |

Russet doesn't clear the environment of the tools that it runs. For example,
curl and Git still read their own environment variables, such as proxy
settings. Russet's downloader doesn't use a proxy, so if you set a proxy
variable such as `https_proxy`, Russet downloads files with curl.

## Related pages

- [Preferences](/reference/preferences/)
- [Run recipes](/guides/run-recipes/)
