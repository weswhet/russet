# Native archive installation

Russet hasn't published a release yet, so these installers come with archives
that you build from source. Try an installation in a disposable VM or a staging
directory before you install on a production computer.

The archive contains native binaries and installation scripts, without a Python
runtime. Native system tools remain dependencies of processors that use them.

## macOS and Linux

Extract the archive, then run its `install.sh`. System installation requires root:

```sh
sudo ./install.sh install
```

On macOS, the installer uses `/Library/AutoPkg` and the original launchd labels,
socket paths, and `/Library/LaunchDaemons` filenames. It reloads both helpers.
On Linux, it uses `/usr/local/lib/autopkg`. Both install
`/usr/local/bin/autopkg` as a symlink to the native executable.

The prior installation, executable/symlink, and macOS launchd files are moved
into a uniquely named rollback generation. They are not deleted. Each upgrade
retains another generation, so rollback can restore earlier native or Python
installations. Preferences, recipe repositories, and caches are untouched.

To roll back on macOS:

```sh
sudo /Library/AutoPkg/install.sh rollback
```

On Linux, use `/usr/local/lib/autopkg/install.sh rollback`. The retired native
installation is retained in the rollback generation for inspection. A failed
installation attempts to restore the original files and services before exiting
with an error. Do not delete rollback generations until they are no longer needed.

For an unprivileged filesystem test, create an empty directory and pass
`--root /absolute/staging/path`. No launchd service is changed when a staging root
is supplied. `--platform Darwin` or `--platform Linux` can exercise that platform's
directory layout in a staging root. A staging test does not prove live helper
behavior or system installation.

## Windows

Extract the archive and run its PowerShell installer with an explicit destination:

```powershell
./install.ps1 install -Destination 'C:\Program Files\AutoPkg'
```

Use an elevated shell when the destination requires administrator access. The
parent directory must already exist. The native command is
`C:\Program Files\AutoPkg\autopkg.exe`; use its full path or add that directory
to PATH. The installer does not alter PATH or change existing Chocolatey shims.

Rollback restores the previous directory, including its original permissions:

```powershell
& 'C:\Program Files\AutoPkg\install.ps1' rollback -Destination 'C:\Program Files\AutoPkg'
```

Each prior version remains in a separate sibling rollback generation. No Python
runtime is needed to install, upgrade, run, or roll back the native archive.

If PowerShell is forcibly terminated during an install or rollback, keep the
extracted archive. A journal in the sibling `AutoPkg-rollbacks` directory records
the pending transaction before either directory is moved. Further install and
rollback commands refuse to proceed until recovery finishes. Run recovery from
the **extracted archive**, because the destination directory may be temporarily
absent:

```powershell
& 'C:\Downloads\autopkg-rs-development-x86_64-pc-windows-msvc\install.ps1' recover -Destination 'C:\Program Files\AutoPkg'
```

Recovery restores the prior installation for an uncommitted install, keeps an
already committed install, or completes an interrupted rollback. It retains the
candidate or retired native files in the generation directory. Recovery can be
repeated if it is interrupted too. Do not manually delete `pending.json` or move
rollback generations. Recovery rejects generation paths outside the expected
history directory and paths crossing reparse points; an occupied conflicting
destination causes an error while preserving both generations. Installer
invocations are serialized with an exclusive file handle that the operating
system releases when the process terminates.

For exact-commit release gates and draft promotion, see [the release-promotion guide](https://github.com/weswhet/russet/blob/main/rust/distribution/RELEASE.md).
