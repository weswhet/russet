---
title: Switch from Python AutoPkg
description: Install Russet next to Python AutoPkg on a Mac, check your recipes, and move scheduled runs to Russet.
---

This page shows you how to install Russet next to Python AutoPkg on a Mac,
check that your recipes and overrides work, and move your scheduled runs to
Russet. Russet doesn't change Python AutoPkg, so you can go back to it at any
time.

## Before you begin

- Make a recipe list file that names every recipe that you run, one per line.
  If your scheduled runs already use a recipe list, use that file. For the
  format, see [Run recipes](/guides/run-recipes/#run-a-list-of-recipes).
- Make sure that you can run commands with `sudo`.

## What Russet shares with Python AutoPkg

Russet uses the same preference keys and file formats as Python AutoPkg, but
it keeps its own preferences in the `com.github.russet` domain. Russet doesn't
read Python AutoPkg's preferences when it runs recipes, so you copy them once
with `russet migrate`. For the steps, see
[Copy your preferences to Russet](#copy-your-preferences-to-russet).

After you copy your preferences, Russet uses the following items without
changes:

- Your recipe repositories, overrides, and their trust information, in the
  folders that your preferences name.
- Your cache, if your preferences set `CACHE_DIR`, including the download
  metadata that avoids repeat downloads.

When a preference doesn't set a folder, Russet uses its own default under
`~/Library/Russet`, not Python AutoPkg's folder under `~/Library/AutoPkg`.
For the defaults, see
[Files and paths](/reference/files-and-paths/#working-folders).

Russet saves preference changes, such as the recipe repositories that
`russet repo-add` adds, to its own domain. It never changes Python AutoPkg's
preferences, so Python AutoPkg doesn't see these changes.

Russet keeps its own installation separate. It installs in `/opt/russet`,
adds the `russet` command at `/usr/local/bin/russet`, and runs its own helper
services. Python AutoPkg's `/Library/AutoPkg` folder, its `autopkg` command,
and its `autopkgserver` and `autopkginstalld` services stay unchanged. The
`russet` command accepts the same verbs and options as the `autopkg` command.

Russet and Python AutoPkg can run side by side. If both tools use the same
cache, don't run the same recipe with both tools at the same time.

Russet doesn't run custom processors that recipe repositories supply in
Python. For the processors that Russet implements, see
[Processors](/reference/processors/).

## Install Russet

Build and package Russet, and then follow the macOS installation steps. For
details, see [Install Russet](/get-started/install/). The installer doesn't
change Python AutoPkg, your preferences, your recipes, or your cache.

To confirm that the installer added the `russet` command, run the following command:

```sh
russet version
```

The output is `3.0.0`, the AutoPkg version that Russet is compatible with.

## Copy your preferences to Russet

Russet starts with empty preferences. To bring your Python AutoPkg settings
over, copy them into Russet's preferences:

1. See what Russet would copy:

   ```sh
   russet migrate --dry-run
   ```

   Each line of the output names a preference key and what would happen to
   it, such as `copied` or `unchanged`. Russet doesn't write anything.

1. Copy the preferences:

   ```sh
   russet migrate
   ```

1. Confirm that Russet reads your preferences:

   ```sh
   russet info
   ```

   The output starts with `Current preferences:` and lists the keys that
   Russet copied.

`russet migrate` copies preferences only. Nothing on disk moves. Paths such
as `RECIPE_REPOS` and `RECIPE_SEARCH_DIRS` that point into `~/Library/AutoPkg`
keep pointing there, so Russet uses the recipe repositories and overrides that
you already have. Python AutoPkg's preferences don't change, so Python AutoPkg
keeps working side by side with Russet.

If you change Python AutoPkg's preferences later, Russet doesn't see the
change. Run `russet migrate` again to copy keys that Russet doesn't have, or
`russet migrate --force` to replace values that differ. For details, see
[`russet migrate`](/reference/cli/russet-migrate/).

## Check your overrides and recipes

Before you rely on Russet, check that it can run your recipes. Run the
following commands from the folder that holds your recipe list:

1. Verify the trust information of your overrides:

   ```sh
   russet verify-trust-info --recipe-list RECIPE_LIST
   ```

   Replace `RECIPE_LIST` with the path to your recipe list file. Each line of
   the output ends with `OK` or `FAILED`.

1. Run the check phase of every recipe:

   ```sh
   russet run --check --recipe-list RECIPE_LIST
   ```

   If a recipe uses a processor that Russet doesn't implement, Russet runs
   none of the recipes and prints the following error:

   ```text
   Custom or unknown processor 'NAME' is not supported
   ```

   Remove that recipe from the list, or find a recipe that uses only supported
   processors, and run the command again.

1. Run a few recipes completely, and compare the results with what Python
   AutoPkg produced, such as the packages that it built or the pkginfo files
   that it imported.

## Move scheduled runs to Russet

Scheduled jobs keep running Python AutoPkg until you change them. In each job,
replace `autopkg` with `russet`, or `/usr/local/bin/autopkg` with
`/usr/local/bin/russet`. The verbs and options stay the same. For an example,
see [Schedule recipe runs](/guides/schedule-runs/).

Check how each job handles exit statuses. If Russet can't find a recipe,
or a recipe fails trust verification or uses an unsupported processor, Russet
runs none of the recipes and exits with status `1`. For details, see
[Exit codes](/reference/exit-codes/).

## Go back to Python AutoPkg

If Russet doesn't work for your recipes, change your scheduled jobs back to
`autopkg`. Python AutoPkg is still installed, so it works as it did before.
To remove Russet as well, see [Roll back or remove Russet](/guides/roll-back/).

## What's next

- [Compatibility with AutoPkg](/concepts/compatibility/)
- [Schedule recipe runs](/guides/schedule-runs/)
- [Troubleshooting](/resources/troubleshooting/)
