# Russet

Russet is an independent project with a native Rust implementation of AutoPkg's recipe engine, CLI, and
46 built-in processors. It targets macOS, Windows, and Linux without bundling
Python or using a Python bridge. Native system tools remain dependencies for
operations such as packaging, disk-image mounting, and signature verification.

Russet hasn't published a release yet; the first release will be 0.1.0. Until
then, build an archive from source as the
[installation guide](https://weswhet.github.io/russet/get-started/install/)
describes. The included installer installs the native command as `russet` and
supports rollback to the previous installation.

Compatibility targets AutoPkg commit `c36e58f` and Munki `7.2.0.5787`. Existing
recipes using supported built-ins retain their processor names and formats.
Custom Python processors and non-FileRepo Munki backends are unsupported and are
rejected before recipe execution. Operations that require a platform-specific
tool remain limited to that platform; no compatibility promise is made for
older operating-system images outside the CI matrix.

Russet distribution versions are independent of the AutoPkg compatibility version.
For recipe compatibility, `russet version` reports **3.0.0**. A release
archive's `RELEASE.json` records its build identity and validation provenance.

Russet also includes native ports of 12 primary-repository processors and one
shared-recipe alias, for 58 implementations under 59 names. See
[community processor support](compatibility/community-processors.md) for the
pinned sources and compatibility boundaries.

## Source and development

The Rust implementation lives in [`rust/`](rust/), and the frozen compatibility
contract that it builds against lives in [`compatibility/`](compatibility/).
[`Scripts/`](Scripts/) holds the shell and PowerShell checks that CI runs on
installed archives. This repository contains no Python.

The suites that compare Russet with Python AutoPkg live in
[russet-compat](https://github.com/weswhet/russet-compat), along with their
results, such as the
[live upstream recipe results](https://github.com/weswhet/russet-compat/blob/main/evidence/live-recipes-2026-10-07.md)
for 228 recipe attempts and the
[94-recipe follow-up](https://github.com/weswhet/russet-compat/blob/main/evidence/community-live-recipes-2026-10-07.md)
for the community processors.

The [Russet documentation](https://weswhet.github.io/russet/) covers
installation, recipe workflows, Munki imports, and the command-line reference.
Its source lives in [`website/`](website/); see its
[README](website/README.md) to build and preview it.

All tests run on GitHub-hosted Actions runners. The supported runner images are
`macos-15`, `macos-15-intel`, `windows-2025`, and `ubuntu-24.04`. See
[Contributing](CONTRIBUTING.md) for the validation and release workflows.

## Attribution and license

Russet implements AutoPkg's recipe interface, and its compatibility contract is
derived from [AutoPkg](https://github.com/autopkg/autopkg)'s Apache-2.0 source.
The project uses the [Apache License 2.0](LICENSE.txt). Vendored dependencies
retain their own license files, which are also included in distribution
archives.
