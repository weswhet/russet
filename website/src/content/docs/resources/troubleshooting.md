---
title: Troubleshooting
description: Causes and fixes for common Russet errors.
---

This page lists common errors, what causes them, and how to fix them. To see
more detail about a failed run, run the recipe again with `-v`, `-vv`, or
`-vvv`.

## Recipe not found

**Symptom:** `russet run` or `russet install` prints the following error
and runs no recipes:

```text
Could not find parent recipe NAME
```

**Cause:** Russet looks for a short name at the top level of each override
and search folder and in the folders one level down. The recipe isn't in any
of those places, usually because you haven't added the recipe repository
that contains it. The error uses the word "parent" even when `NAME` is the
recipe that you asked for.

**Resolution:** add the recipe repository that contains the recipe. For
details, see [Add recipe repositories](/guides/add-recipe-repositories/).

## Overrides rejected by trust verification

**Symptom:** an override fails with the following error:

```text
Trust records must come from a local override outside recipe repositories
```

**Cause:** the override is inside a recipe repository: the `RECIPE_REPO_DIR`
folder or a repository in `RECIPE_REPOS`. This happens when an override
folder, from `RECIPE_OVERRIDE_DIRS` or `--override-dir`, points into a cloned
repository.

**Resolution:** keep your overrides in a folder outside your recipe
repositories, such as `~/Library/AutoPkg/RecipeOverrides`. For details, see
[Recipe trust](/concepts/recipe-trust/#where-trusted-overrides-must-be).

## Unsupported processor

**Symptom:** `russet run` prints the following error and runs no recipes:

```text
Custom or unknown processor 'NAME' is not supported
```

**Cause:** a recipe in the command, or one of its parent recipes, uses a
processor that Russet doesn't implement. This is usually a custom processor
that a recipe repository supplies in Python.

**Resolution:** remove the recipe from the command or recipe list, or use a
different recipe for the same software. Check the
[processor list](/reference/processors/) to see what Russet implements.

## Parent recipe changed

**Symptom:** `russet run` or `russet verify-trust-info` reports the
following error:

```text
Parent recipe RECIPE_IDENTIFIER contents differ from expected
```

**Cause:** the parent recipe changed after you created or last updated the
override, usually because `russet repo-update` pulled a new version.

**Resolution:** review the change, and if you trust it, run
`russet update-trust-info OVERRIDE`. For the steps, see
[Create recipe overrides](/guides/create-overrides/#review-a-change-and-update-trust-information).

## Can't save preferences

**Symptom:** on Linux or Windows, `russet repo-add` or `russet repo-delete`
prints the following error:

```text
No writable preference file loaded; use --prefs FILE
```

`russet repo-add` clones the repository before it prints this error, but it
doesn't add the repository to your search folders.

**Cause:** Russet saves preferences only to a preference file that it loaded,
and it ignores a missing or empty file.

**Resolution:** create a preference file with at least one key, and run the
command again. For details, see
[Configure preferences](/guides/configure-preferences/).

## Operation needs macOS

**Symptom:** a recipe fails on Linux or Windows with an error such as one of
the following:

```text
Package creation and installation are only supported on macOS
Code signature verification is only supported on macOS.
```

**Cause:** the recipe uses a processor that needs macOS tools.

**Resolution:** run the recipe on a Mac. To see which operations each platform
supports, see [Platform support](/concepts/platform-support/).

## Disk image can't be read on Linux

**Symptom:** a recipe fails on Linux with an error that starts with
`mounting` and names the image, such as `is an encrypted disk image` or
`isn't a disk image that Russet can open natively`.

**Cause:** Russet's built-in reader doesn't support the image. It reads
read-only `.dmg` images with HFS+ or APFS volumes, but not encrypted images or
ISO images.

**Resolution:** run the recipe on a Mac. If the error mentions free space, set
`RUSSET_SCRATCH_DIR` to a folder with room for the image's contents.

## Helper service unavailable

**Symptom:** a recipe that builds a package fails with an error such as the
following:

```text
Couldn't connect to /var/run/russet-server: No such file or directory (os error 2)
```

An install recipe can finish without installing anything. With `-v`, its
output includes a line that starts with
`Result: ERROR: Couldn't connect to /var/run/russet-installd`.

**Cause:** the helper service's launchd job isn't loaded.

**Resolution:** check whether launchd has the jobs:

```sh
sudo launchctl print system/com.github.weswhet.russet.server
sudo launchctl print system/com.github.weswhet.russet.installd
```

If a command reports that it can't find the service, load the job. For
example, to load the package builder, run the following command:

```sh
sudo launchctl bootstrap system /Library/LaunchDaemons/com.github.weswhet.russet.server.plist
```

If launchd has the job but requests still fail, read the service's log in
`/private/var/log/russet-server` or `/private/var/log/russet-installd`. A
helper service refuses to start if `/opt/russet/russet` or a folder that
contains it isn't owned by root or is writable by other users. Installing Russet again
restores the expected ownership and permissions.

## macOS refuses to run russet

**Symptom:** after you install Russet, macOS shows an alert that it can't
open `russet` when you run the command.

**Cause:** the archive came from another computer, for example through a web
browser, and macOS marked it with a quarantine attribute that the installed
files kept. Archives that you build from source aren't notarized.

**Resolution:** build and package Russet on the Mac where you install it, and
then install it again. For the steps, see
[Install Russet](/get-started/install/#build-and-package-russet).

## Unsupported Munki repository

**Symptom:** every recipe fails before it runs, including recipes that don't
use Munki, with the following error:

```text
Only Munki FileRepo repositories are supported
```

**Cause:** the `MUNKI_REPO_PLUGIN` preference names a backend other than
`FileRepo`, or `MUNKI_REPO` is a URL other than a `file://` URL.

**Resolution:** delete the `MUNKI_REPO_PLUGIN` preference or set it to
`FileRepo`, and set `MUNKI_REPO` to a local path. If the repository is on a
file share, mount the share and use the mount path.

## Recipe needs a newer AutoPkg version

**Symptom:** a recipe fails to load with an error such as the following:

```text
Recipe requires at least autopkg version 3.1, but this implementation targets 3.0.0.
```

**Cause:** the recipe's `MinimumVersion` is higher than Russet's
compatibility version, 3.0.0.

**Resolution:** use a version of the recipe that supports AutoPkg 3.0.0, or
wait for a Russet release that targets a newer AutoPkg version.

## Recipe has no check phase

**Symptom:** `russet run --check` prints the following error and runs no
recipes:

```text
Recipe at PATH is missing EndOfCheckPhase Processor, not possible to perform check.
```

**Cause:** a recipe in the command doesn't have an `EndOfCheckPhase` step, so
Russet can't tell where its check phase ends.

**Resolution:** remove the recipe from the check run, or run it without
`--check`.

## GitHub requests fail

**Symptom:** recipes that use `GitHubReleasesInfoProvider`, or
`russet search`, fail with HTTP 403 errors after several runs.

**Cause:** GitHub limits the number of API requests that it accepts without
authentication.

**Resolution:** create a GitHub personal access token, and save it in
`~/.autopkg_gh_token` or in the `GITHUB_TOKEN` preference. Russet reads the
token only from these locations. For details, see
[Preferences](/reference/preferences/).

## What's next

- [Exit codes](/reference/exit-codes/)
- [Compatibility with AutoPkg](/concepts/compatibility/)
