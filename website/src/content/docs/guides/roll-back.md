---
title: Roll back or remove Russet
description: Restore the installation that Russet replaced, recover from an interrupted installation, and remove Russet.
---

This page shows you how to restore the installation that Russet replaced, how
to recover from an interrupted installation on Windows, and how to remove
Russet from a computer.

## Before you begin

- Make sure that you can run commands as an administrator.

## How rollback works

Each time you install Russet, the installer moves the files that it replaces
into a new rollback generation instead of deleting them. A rollback restores
the files from the most recent generation and moves the Russet files that it
replaces into that generation's `retired` folder.

A rollback goes back one installation at a time. If you installed Russet
twice, the first rollback restores the earlier Russet installation, and a
second rollback restores what was there before that. Rollback doesn't change
your preferences, recipes, or cache.

## Roll back on macOS

To restore the previous installation on macOS, run the following command:

```sh
sudo /Library/AutoPkg/install.sh rollback
```

The installer stops the helper services, restores the previous `autopkg`
command, helper services, and launchd jobs, and starts the services again.
When it finishes, it prints where it kept the Russet files:

```text
Restored previous installation; retired native files retained in /Library/AutoPkg-Rollbacks/generation.XXXXXXXX/retired
```

If the previous installation was Python AutoPkg, the restored installation
has no rollback record, so you can't roll back further from it. To return to
Russet, install it again.

## Roll back on Linux

To restore the previous installation on Linux, run the following command:

```sh
sudo /usr/local/lib/autopkg/install.sh rollback
```

You can also run `install.sh rollback` from the extracted archive folder.

## Roll back on Windows

To restore the previous installation on Windows, run the installer copy in
the installation folder:

```powershell
& 'DESTINATION\install.ps1' rollback -Destination 'DESTINATION'
```

Replace `DESTINATION` with the installation folder, such as
`C:\Tools\AutoPkg`. Use an elevated PowerShell session if the folder needs
administrator access.

## Recover an interrupted installation on Windows

If something interrupts an installation or a rollback on Windows, the
installer refuses to install or roll back until you recover. To finish or undo the
interrupted change, run the `recover` action from the extracted archive:

```powershell
& '.\ARCHIVE_FOLDER\install.ps1' recover -Destination 'DESTINATION'
```

Replace the following:

- `ARCHIVE_FOLDER`: the extracted archive folder, such as
  `autopkg-rs-development-x86_64-pc-windows-msvc`.
- `DESTINATION`: the installation folder.

On macOS and Linux, if an installation or rollback stops because of an error
or a signal, such as Control-C, the installer restores the previous files
before it exits.

## Remove Russet

Russet doesn't have an uninstaller. On a computer that didn't have AutoPkg
before you installed Russet, rolling back each Russet installation removes
the `autopkg` command. On macOS, it also removes the helper services and
their launchd jobs.

The rollback history stays on disk so that you can inspect it. After you
confirm that you don't need it, delete it:

- **macOS:** `/Library/AutoPkg-Rollbacks`
- **Linux:** `/usr/local/lib/autopkg-rollbacks`
- **Windows:** the `-rollbacks` folder next to the installation folder

Rollback doesn't remove your preferences, recipe repositories, overrides, or
cache. To remove them, delete the folders listed in
[Files and paths](/reference/files-and-paths/#working-folders), and your
preferences. For preference locations, see
[Preferences](/reference/preferences/#preference-locations).

:::danger
Deleting the rollback history removes the only copy of the installations that
Russet replaced. You can't roll back after you delete it.
:::

## What's next

- [Install Russet](/get-started/install/)
- [Files and paths](/reference/files-and-paths/)
