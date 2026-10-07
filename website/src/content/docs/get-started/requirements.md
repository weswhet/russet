---
title: System requirements
description: The platforms, build tools, runtime tools, and permissions that Russet needs.
---

This page lists the platforms that Russet supports and the tools and
permissions that it needs to build, install, and run.

## Supported platforms

Russet builds and runs on the following platforms:

| Platform | Architecture | Target |
| --- | --- | --- |
| macOS | Apple silicon | `aarch64-apple-darwin` |
| macOS | Intel | `x86_64-apple-darwin` |
| Linux | x86-64 | `x86_64-unknown-linux-gnu` |
| Windows | x86-64 | `x86_64-pc-windows-msvc` |

Continuous integration builds and tests Russet on the GitHub-hosted runner
images `macos-15`, `macos-15-intel`, `ubuntu-24.04`, and `windows-2025`. These
images are the tested baselines, not minimum supported versions. Russet makes no
compatibility promise for operating system versions outside that matrix.

Some processors work only on the platform whose tools they use. For example,
mounting disk images, building packages, and verifying code signatures work
only on macOS. For details, see [Platform support](/concepts/platform-support/).

## Build tools

Russet doesn't have a published release yet, so you
[build it from source](/get-started/install/). To build it, you need the
following tools:

- **Git**, to clone the Russet repository.
- **Rust**, the current stable release. To install it, use `rustup`, as the
  [Rust installation page](https://www.rust-lang.org/tools/install)
  describes.
- **A C compiler and linker**, because Rust uses the system linker, and one of
  Russet's dependencies includes C code:
  - **macOS:** the Xcode Command Line Tools. To install them, run
    `xcode-select --install`.
  - **Linux:** a C compiler, such as `gcc`. On Debian and Ubuntu, install
    the `build-essential` package.
  - **Windows:** the Microsoft C++ Build Tools. The Rust installer offers to
    install them.

## Required tools

Russet doesn't bundle or use Python. It runs native system tools for some
operations:

- **curl:** Russet uses curl for downloads, GitHub API requests, and
  `autopkg search`. macOS and Windows include curl. On Linux, install curl
  from your distribution's package manager.
- **Git:** `autopkg repo-add` and `autopkg repo-update` clone and update
  recipe repositories with Git. Trust information also records Git details
  when Git is available. Install Git on every platform where you manage
  recipe repositories.
- **macOS system tools:** on macOS, Russet uses tools that ship with the
  operating system, such as `hdiutil`, `pkgbuild`, `installer`, `pkgutil`,
  `codesign`, and `ditto`. You don't need to install them.
- **Windows tools for specific processors:** `ChocolateyPackager` needs the
  Chocolatey command `choco.exe`, and `SignToolVerifier` needs `signtool.exe`
  from the Windows SDK. Other processors don't need them.

## Permissions

The installers change system locations, so they need administrator access:

- **macOS and Linux:** run `install.sh` with `sudo`. A system installation
  requires the root user.
- **Windows:** run `install.ps1` from an elevated PowerShell session when the
  destination folder needs administrator access, such as a folder under
  `C:\Program Files`.

You don't need administrator access to run recipes. On macOS, privileged
helper services build packages and install software for recipes that need
them. For details, see [How Russet works](/concepts/how-russet-works/).

## Disk space

Russet stores downloads and build products in its cache, which is
`~/Library/AutoPkg/Cache` by default. Make sure that the cache's volume has
room for the software that your recipes download. To change the location, see
[Preferences](/reference/preferences/).

## What's next

- [Install Russet](/get-started/install/)
- [Platform support](/concepts/platform-support/)
