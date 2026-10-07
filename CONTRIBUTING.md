# Contributing to Russet

Russet is an independent Rust implementation of the AutoPkg compatibility
contract. Make implementation changes in [`rust/`](rust/), and keep the frozen
contract in [`compatibility/`](compatibility/) in sync with the capture scripts
in [russet-compat](https://github.com/weswhet/russet-compat). This repository
contains no Python; don't add Python code or a Python runtime dependency.

Preserve existing recipe names, formats, processor manifests, input precedence,
reports, trust records, and error behavior. Keep platform restrictions explicit.
Russet does not support custom Python processors, Python bridges, or Munki
backends other than FileRepo.

## Validate changes in GitHub Actions

Run regression suites on GitHub-hosted Actions runners. Do not run test suites
on local virtual machines or self-hosted runners. Edit and review source locally,
then push a branch and open a pull request to obtain validation results.

This repository defines these workflows:

| Workflow | Purpose |
| --- | --- |
| [Rust development](.github/workflows/rust.yml) | Formatting, strict Clippy checks, workspace tests, installer tests, builds, packaging, and installed-archive checks on all four targets. |
| [Docs](.github/workflows/docs.yml) | Builds the documentation site for pull requests and publishes it to GitHub Pages from `main`. |
| [Rust verified draft release](.github/workflows/rust-release.yml) | Verifies release evidence and prepares distribution archives and a draft release. |

Relevant pushes and pull requests trigger validation automatically. Each workflow
also supports manual dispatch from the repository's **Actions** tab. If a change
falls outside a workflow's path filters, dispatch that workflow explicitly.
Inspect every required platform job and attach the run links to your pull request.

## Compare with Python AutoPkg

The comparison suites run in
[russet-compat](https://github.com/weswhet/russet-compat). Its workflows build
Russet from a `weswhet/russet` ref and compare it with Python AutoPkg on macOS,
Linux, and Windows. To check a change, dispatch them with your branch or commit
as `russet_ref`.

For behavior changes, add cases there that compare final environment values,
failures, reports, generated metadata, filesystem effects, or subprocess
arguments as appropriate. Use isolated fixtures for each implementation; never
compare Python and Rust against the same mutable Munki repository. Normalize
only documented nondeterministic fields. When a case needs a frozen Python
result inside Russet's own tests, capture it once and commit it as a fixture.

## Prepare a release

1. Run **Rust development** for the intended source commit and wait for all four
   targets to pass.
2. Dispatch the russet-compat workflows with that commit as `russet_ref`, and
   review the results.
3. Dispatch **Rust verified draft release** at that commit with its full SHA,
   the distribution version, and the development run ID.
4. Review the draft's assets, checksums, provenance, and installation
   instructions before publication.

See [the release-promotion guide](rust/distribution/RELEASE.md) for the gates
that promotion checks. Retain license and attribution notices when changing or
redistributing source. The distribution version is separate from the
compatibility version reported by `autopkg version`.
