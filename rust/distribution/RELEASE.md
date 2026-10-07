# Promote a verified native distribution

Release promotion reuses four tested development archives. It doesn't rebuild
binaries, install anything on the host, or publish a release. The manual
workflow creates a **draft** for review.

Distribution versions use `MAJOR.MINOR.PATCH`. The first Russet release is
`0.1.0`, with tag `rust-v0.1.0` and archives named `russet-0.1.0-TARGET`. The
distribution version identifies the native distribution. The CLI and recipe
environment keep AutoPkg compatibility version `3.0.0`, and Cargo's internal
package version is independent of both.

## Required evidence

Promotion needs one successful run of `rust.yml` in `weswhet/russet` at the full
source SHA. The run must come from a push or a manual dispatch; pull-request
runs can't promote releases. All four target jobs must pass formatting, Clippy,
workspace tests, installer tests, release builds, packaging, the native
installed workflow checks, and the archive upload.

The promoter requires the expected jobs and steps explicitly; skipped required
steps are failures. It verifies the SHA-256 digests of the downloaded GitHub
artifacts. Missing, duplicate, expired, cross-commit, or mismatched artifacts
fail before a release output directory is published. Workflow step names are
part of this gate contract: update the promoter and its tests when you rename a
required step, including when a dependency update changes an action's version
in a step name such as `Run actions/upload-artifact@v4`.

The development jobs supply native installation, upgrade, rollback, helper, and
Python-free runtime checks. Comparisons with Python AutoPkg run separately in
[russet-compat](https://github.com/weswhet/russet-compat); before you promote a
release, dispatch its workflows with the release commit as `russet_ref` and
review the results. Promotion doesn't imply compatibility with operating systems
outside the runner baselines.

## Local preparation

Use a Rust toolchain and an authenticated `gh` CLI with read access to the
repository's Actions. Choose a new output directory, and then run this command
from `rust/`:

```sh
cargo xtask promote \
  --commit FULL_SOURCE_SHA --version 0.1.0 \
  --development-run DEVELOPMENT_RUN_ID \
  --output release-output
```

The output contains four archives, `gate-evidence.json`, and `SHA256SUMS`.
Each archive contains `RELEASE.json`, installation instructions, and a release
README. Original candidate documentation remains under `DEVELOPMENT-*` names.
All original binaries, installer scripts, launchd configurations, license files,
and their permission modes are kept without modification. Archive root names and
documentation change, so promoted archive hashes differ from the original
development archive hashes, which remain recorded in the metadata.

## Draft workflow and publication boundary

Select **Rust verified draft release** in Actions at the exact source commit,
and provide that full SHA, the distribution version, and the successful
development run ID. The verification job has read-only repository permissions.
A dependent job with contents-write permission creates the draft and uploads the
verified artifacts. An existing tag or release isn't overwritten.

Review the draft and evidence before any separate decision to publish. This
workflow never publishes a release and never installs the candidate on the
machine that runs it.

## Validation

Run `cargo test --package xtask --bin xtask` from `rust/`. The tests use
isolated synthetic archives and mocked GitHub responses; they don't contact
GitHub or create releases. They cover failed and skipped gates, stale artifacts,
source provenance, archive identity and hashes, unsafe members, payload
preservation, reproducibility, existing output directories, and atomic output.
The installer transaction tests run with `cargo test --package xtask`.
