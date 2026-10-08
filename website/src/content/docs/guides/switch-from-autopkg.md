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

Russet uses the same preference domain, working folders, and file formats as
Python AutoPkg, so it uses the following items without changes:

- Your preferences in the `com.github.autopkg` domain.
- Your recipe repositories, overrides, and their trust information.
- Your cache, including the download metadata that avoids repeat downloads.

Russet keeps its own installation separate. It installs in `/opt/russet`,
adds the `russet` command at `/usr/local/bin/russet`, and runs its own helper
services. Python AutoPkg's `/Library/AutoPkg` folder, its `autopkg` command,
and its `autopkgserver` and `autopkginstalld` services stay unchanged. The
`russet` command accepts the same verbs and options as the `autopkg` command.

Because both tools share the cache, don't run the same recipe with both tools
at the same time.

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
