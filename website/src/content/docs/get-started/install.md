---
title: Install Russet
description: Build Russet from source, package it, and install the russet command on macOS, Linux, or Windows.
---

This page shows you how to build Russet from source, package the build into
an archive, and install the `russet` command from that archive. The installer
keeps your previous installation, including Python AutoPkg, so that you can
roll back to it.

:::note
Russet doesn't have a published release yet. Prebuilt archives start with
Russet 0.1.0. Until then, build Russet from source as this page describes.
:::

## Before you begin

- Check that your computer meets the
  [system requirements](/get-started/requirements/), and install the
  [build tools](/get-started/requirements/#build-tools).
- Make sure that you can run commands as an administrator.

## Build and package Russet

Build Russet on the computer where you plan to install it. The packaging step
checks that the programs match the platform that you name, and an archive that
you copy from another computer can need extra steps before macOS runs it.

To build and package Russet, follow these steps. On Windows, run the commands
in PowerShell.

1. Clone the Russet repository, and then go to its `rust` folder:

   ```sh
   git clone https://github.com/weswhet/russet.git
   cd russet/rust
   ```

1. Build the `russet` command, which also runs the macOS helper services:

   ```sh
   cargo build --release --locked -p russet
   ```

   The first build downloads the Rust dependencies, so it can take several
   minutes.

1. Package the build into an archive:

   ```sh
   cargo xtask package --target TARGET --bin-dir target/release
   ```

   Replace `TARGET` with the target for your computer:

   - `aarch64-apple-darwin` for a Mac with Apple silicon.
   - `x86_64-apple-darwin` for a Mac with an Intel processor.
   - `x86_64-unknown-linux-gnu` for Linux on x86-64.
   - `x86_64-pc-windows-msvc` for Windows on x86-64.

   The command creates the archive in the `dist` folder and prints its path.
   The archive's name is `russet-development-TARGET.tar.gz`, or
   `russet-development-TARGET.zip` on Windows. If the programs don't match
   `TARGET`, the command stops with the following error:

   ```text
   Packaging failed: Executable does not match target TARGET
   ```

The remaining steps run from the `rust` folder of the repository.

## Install on macOS

The installer puts Russet in `/opt/russet` and sets up its privileged helper
services with their own launchd jobs and sockets. It doesn't change Python
AutoPkg, so you can keep both on the same Mac.

To install Russet on macOS, follow these steps:

1. Extract the archive into a new folder, and then go to the extracted folder.
   For a Mac with Apple silicon, run the following commands:

   ```sh
   mkdir -p ~/russet-install
   tar -xzf dist/russet-development-aarch64-apple-darwin.tar.gz -C ~/russet-install
   cd ~/russet-install/russet-development-aarch64-apple-darwin
   ```

   For a Mac with an Intel processor, replace `aarch64-apple-darwin` with
   `x86_64-apple-darwin`.

1. Run the installer:

   ```sh
   sudo /bin/sh ./install.sh install
   ```

   The installer stops the Russet helper services, moves any existing Russet
   installation aside, installs the new files, and starts the services again.
   When it finishes, it prints the rollback command:

   ```text
   Installed Russet; rollback: /opt/russet/install.sh rollback
   ```

1. Keep the extracted folder until you've checked your recipes. You can roll
   back from it or from `/opt/russet/install.sh`.

The installer creates the following items:

- `/opt/russet/russet`, with a symbolic link at
  `/usr/local/bin/russet`.
- `/Library/LaunchDaemons/com.github.weswhet.russet.server.plist` and
  `/Library/LaunchDaemons/com.github.weswhet.russet.installd.plist`, the
  launchd jobs that run `russet --server` and `russet --installd` as the
  privileged helper services.
- `/opt/russet-rollbacks`, which holds the files that each installation
  replaced.

## Install on Linux

To install Russet on Linux, follow these steps:

1. Extract the archive into a new folder, and then go to the extracted folder:

   ```sh
   mkdir -p ~/russet-install
   tar -xzf dist/russet-development-x86_64-unknown-linux-gnu.tar.gz -C ~/russet-install
   cd ~/russet-install/russet-development-x86_64-unknown-linux-gnu
   ```

1. Run the installer:

   ```sh
   sudo /bin/sh ./install.sh install
   ```

The installer creates `/opt/russet/russet`, with a symbolic link at
`/usr/local/bin/russet`. It keeps replaced files in `/opt/russet-rollbacks`.
Linux doesn't use the helper services.

Before you add recipe repositories on Linux, create a preference file. For
details, see [Configure preferences](/guides/configure-preferences/).

## Install on Windows

The Windows installer copies `russet.exe` into a folder that you choose. It
doesn't change your `PATH` or any Chocolatey shims.

To install Russet on Windows, follow these steps:

1. In PowerShell, extract the archive into a new folder:

   ```powershell
   Expand-Archive .\dist\russet-development-x86_64-pc-windows-msvc.zip -DestinationPath "$HOME\russet-install"
   ```

1. Run the installer. If the destination needs administrator access, use an
   elevated PowerShell session.

   ```powershell
   & "$HOME\russet-install\russet-development-x86_64-pc-windows-msvc\install.ps1" install -Destination 'C:\Tools\Russet'
   ```

   The parent of the destination folder must already exist. The installer
   moves any existing destination folder into
   `C:\Tools\Russet-rollbacks`, so don't choose a folder that holds other
   files.

1. To run `russet` from any folder, add the destination folder to your
   `PATH`. For example, to add it for the current PowerShell session, run the
   following command:

   ```powershell
   $env:Path += ';C:\Tools\Russet'
   ```

If something interrupts an installation or rollback, run the `recover` action
from the extracted archive before you try again:

```powershell
& "$HOME\russet-install\russet-development-x86_64-pc-windows-msvc\install.ps1" recover -Destination 'C:\Tools\Russet'
```

## Check the installation

To confirm that your shell runs the installed command, run the following
command:

```sh
russet version
```

The output is the AutoPkg compatibility version:

```text
3.0.0
```

Russet reports the AutoPkg version that it's compatible with, so that recipes
that set `MinimumVersion` keep working. An archive that you build from source
doesn't record a Russet version. To record what you installed, run
`git rev-parse HEAD` in the `russet` folder, and keep the commit that it
prints.

## Update Russet

To update Russet, run `git pull` in the `russet` folder, and then repeat the
steps to build, package, and install it. Each installation keeps the one that
it replaces as a new rollback generation.

## What's next

- [Quickstart](/get-started/quickstart/)
- [Switch from Python AutoPkg](/guides/switch-from-autopkg/)
- [Roll back or remove Russet](/guides/roll-back/)
