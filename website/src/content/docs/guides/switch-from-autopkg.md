---
title: Switch from Python AutoPkg
description: Replace Python AutoPkg with Russet on a Mac, check your recipes, and update scheduled runs.
---

This page shows you how to replace Python AutoPkg with Russet on a Mac, check
that your recipes and overrides work, and update your scheduled runs. If
something doesn't work, you can roll back to Python AutoPkg.

## Before you begin

- Make a recipe list file that names every recipe that you run, one per line.
  If your scheduled runs already use a recipe list, use that file. For the
  format, see [Run recipes](/guides/run-recipes/#run-a-list-of-recipes).
- Make sure that you can run commands with `sudo`.

## What carries over

Russet uses the same preference domain, folders, and file formats as Python
AutoPkg, so the following items keep working without changes:

- Your preferences in the `com.github.autopkg` domain.
- Your recipe repositories, overrides, and their trust information.
- Your cache, including the download metadata that avoids repeat downloads.
- The `autopkg` command path, `/usr/local/bin/autopkg`.
- The helper services' launchd jobs and sockets.

Russet doesn't run custom processors that recipe repositories supply in
Python. For the processors that Russet implements, see
[Processors](/reference/processors/).

## Install Russet

To replace Python AutoPkg, build and package Russet, and then follow the
macOS installation steps. For details, see
[Install Russet](/get-started/install/). The installer stops
the helper services, moves Python AutoPkg's files into a rollback folder,
installs Russet in their place, and starts the services again. It doesn't
change your preferences, recipes, or cache.

Both Python AutoPkg 3.0.0 and Russet report version `3.0.0`. To confirm that
the `autopkg` command is Russet, check the type of the `autopkg` file:

```sh
file /Library/AutoPkg/autopkg
```

For Russet, the output names a Mach-O executable, such as
`Mach-O 64-bit executable arm64`. For Python AutoPkg, the output names a
Python script.

## Check your overrides and recipes

Before you rely on Russet, check that it can run your recipes. Run the
following commands from the folder that holds your recipe list:

1. Verify the trust information of your overrides:

   ```sh
   autopkg verify-trust-info --recipe-list RECIPE_LIST
   ```

   Replace `RECIPE_LIST` with the path to your recipe list file. Each line of
   the output ends with `OK` or `FAILED`.

1. Run the check phase of every recipe:

   ```sh
   autopkg run --check --recipe-list RECIPE_LIST
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

## Update scheduled runs

Scheduled jobs that run `autopkg` keep working after you switch. Check how
each job handles exit statuses. If Russet can't find a recipe,
or a recipe fails trust verification or uses an unsupported processor, Russet
runs none of the recipes and exits with status `1`. For details, see
[Exit codes](/reference/exit-codes/).

## Roll back to Python AutoPkg

If Russet doesn't work for your recipes, restore Python AutoPkg:

```sh
sudo /Library/AutoPkg/install.sh rollback
```

For details, see [Roll back or remove Russet](/guides/roll-back/).

## What's next

- [Compatibility with AutoPkg](/concepts/compatibility/)
- [Schedule recipe runs](/guides/schedule-runs/)
- [Troubleshooting](/resources/troubleshooting/)
