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

On macOS and Linux, the installer uses `/opt/russet` and installs
`/usr/local/bin/russet` as a symlink to the native executable. On macOS, it
also installs the `com.github.weswhet.russet.server` and
`com.github.weswhet.russet.installd` launchd jobs, which run `russet --server`
and `russet --installd` on the `/var/run/russet-server` and
`/var/run/russet-installd` sockets, and reloads both. The installer doesn't
change an existing Python AutoPkg installation, its `autopkg` command, or its
launchd jobs.

The prior Russet installation, the `russet` symlink, and the Russet launchd
files are moved into a uniquely named rollback generation in
`/opt/russet-rollbacks`. They are not deleted. Each upgrade retains another
generation, so rollback can restore earlier native installations. Preferences,
recipe repositories, and caches are untouched.

To roll back:

```sh
sudo /opt/russet/install.sh rollback
```

The retired native installation is retained in the rollback generation for
inspection. A failed installation attempts to restore the original files and
services before exiting with an error. Do not delete rollback generations until they are no longer needed.

For an unprivileged filesystem test, create an empty directory and pass
`--root /absolute/staging/path`. No launchd service is changed when a staging root
is supplied. `--platform Darwin` or `--platform Linux` can exercise that platform's
directory layout in a staging root. A staging test does not prove live helper
behavior or system installation.

## Windows

Extract the archive and run its PowerShell installer with an explicit destination:

```powershell
./install.ps1 install -Destination 'C:\Program Files\Russet'
```

Use an elevated shell when the destination requires administrator access. The
parent directory must already exist. The native command is
`C:\Program Files\Russet\russet.exe`; use its full path or add that directory
to PATH. The installer does not alter PATH or change existing Chocolatey shims.

Rollback restores the previous directory, including its original permissions:

```powershell
& 'C:\Program Files\Russet\install.ps1' rollback -Destination 'C:\Program Files\Russet'
```

Each prior version remains in a separate sibling rollback generation. No Python
runtime is needed to install, upgrade, run, or roll back the native archive.

If PowerShell is forcibly terminated during an install or rollback, keep the
extracted archive. A journal in the sibling `Russet-rollbacks` directory records
the pending transaction before either directory is moved. Further install and
rollback commands refuse to proceed until recovery finishes. Run recovery from
the **extracted archive**, because the destination directory may be temporarily
absent:

```powershell
& 'C:\Downloads\russet-development-x86_64-pc-windows-msvc\install.ps1' recover -Destination 'C:\Program Files\Russet'
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
