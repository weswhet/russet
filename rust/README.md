# Rust implementation: development notes

Russet hasn't published a release yet; the first release will be 0.1.0. This
document covers building, testing, and packaging Russet from this checkout.

All 46 original built-in names and 12 promoted processor implementations are registered,
with one shared-recipe alias (59 names total). See [community support](../compatibility/community-processors.md). Execution routes cover portable file,
download, archive, metadata, Munki, native package, signature, and helper-client
operations. Registration does not establish complete behavior or platform parity.
Custom Python processors and Python Munki backends are rejected before recipe
steps execute. Native system tools remain dependencies; Rust does not load
Python processors or require a host Python runtime. The framework builder runs
Python from its output artifact for pip setup and smoke tests, as documented in
the community processor support notes.

## Build and run

From the repository root:

```sh
cargo build --manifest-path rust/Cargo.toml --workspace
rust/target/debug/russet list-processors
rust/target/debug/russet processor-info FileCreator
env AUTOPKG_RS_CACHE_DIR=/tmp/autopkg-rust-example rust/target/debug/russet run -v rust/examples/files.recipe.yaml
```

Cache paths follow Python AutoPkg: `CACHE_DIR` preferences, then `~/Library/AutoPkg/Cache`, with home expansion and absolute-path normalization. `AUTOPKG_RS_CACHE_DIR` supplies a separate cache when the preference is unset; `--key CACHE_DIR=...` overrides the per-recipe cache. To keep experiments and their run reports isolated, use a separate preference cache or `AUTOPKG_RS_CACHE_DIR`, and a separate mutable Munki repository.
`AUTOPKG_RS_DEBUG=1` enables execution diagnostics on stderr. Standalone
processors read a dictionary plist from stdin and write a dictionary plist to
stdout:

```sh
rust/target/debug/russet processor-run FileCreator < input.plist > output.plist
```

Standalone processor errors exit with code 10; failures corresponding to uncaught
reference exceptions exit with code 1. Recipe execution failures exit with code
70. Successful commands exit with code 0.

## Test

From `rust/`:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --exclude xtask --locked
cargo test --package xtask --locked
```

The workspace tests include real local HTTP servers, package and disk image
tools on macOS, Foundation predicates, signature verification, cache
boundaries, report serialization, and isolated helper sockets. Several tests
compare Russet with outputs frozen from Python AutoPkg, such as the predicate
results in `compatibility/` and the typed YAML, receipt, and Munki catalog
fixtures under each crate's `tests/fixtures/`. Tests that need an installed,
pinned Munki are ignored by default.

The `xtask` tests build development archives and run their installers in
temporary staging roots to check installation, upgrade, rollback, and failure
recovery. The PowerShell installer tests run when `pwsh` is on your `PATH`.
They also cover release promotion with synthetic archives and mocked GitHub
responses.

Russet's repository contains no Python. The suites that compare Russet with
Python AutoPkg 3.0.0, and the scripts that capture the files in
[`compatibility/`](../compatibility/), live in
[russet-compat](https://github.com/weswhet/russet-compat). Its CLI suite covers
179 cases across command parsers, aliases, inspection, execution, reports, and
persistence; this coverage doesn't certify every possible argument or failure
path.

## Components

| Crate | Responsibility |
| --- | --- |
| `russet` | CLI and native command output |
| `autopkg-value` | Typed values, nulls, and explicit property-list serialization |
| `autopkg-engine` | Loading, inheritance, substitution, validation, trust, preferences, caches, and receipts |
| `autopkg-processors` | Explicit built-in registry and sequential processor implementations |
| `autopkg-platform` | Foundation, native tools, mounts, signatures, GitHub, and Chocolatey |
| `autopkg-munki` | FileRepo indexes, metadata, imports, receipt editing, and icons |
| `autopkg-helpers` | Native socket clients and the macOS helper services that `russet --server` and `russet --installd` run |
| `xtask` | Development archives, installer tests, and release promotion; not shipped |

## Known boundaries

- The environment preserves nulls, dates, binary data, and YAML scalar types.
  Some Python string and regular expression edge cases still need broader
  comparison coverage. Typed plist and YAML values are tested through CLI
  execution and persisted receipts. See `crates/engine/README.md` for
  serialization boundaries.
- Reports preserve the diagnostic field with a native backtrace; Python stack
  frames are a documented runtime boundary, not fabricated. Parent recipe and
  package-script trust hashes are supported.
- Munki supports staged installer metadata, mounted images, and bundle packages.
  The pinned option corpus covers every captured declaration and alias.
  Multi-volume mount lifetime tests and native icon fixtures pass; uncommon
  payload formats and non-macOS OS-version metadata remain coverage boundaries.
  See `crates/munki/README.md`.
- Chocolatey `license` and `contentFiles` may be omitted or null. The pinned Python implementation requires Python generator objects for non-null values and rejects ordinary recipe dictionaries and arrays; Rust rejects those inputs explicitly.
- Privileged helpers preserve the existing socket protocol and authorization
  behavior. Tests exercise validation, argument construction, unprivileged
  copying, and isolated sockets.

## Development archives and rollback

CI builds macOS arm64/x86-64, Windows x86-64, and Linux x86-64. Image labels are
selected from the [GitHub runner inventory](https://github.com/actions/runner-images/blob/main/README.md).
No older operating-system compatibility is promised.

To build a development archive, run these commands from `rust/`, replacing
`TARGET` with the Rust target triple, such as `aarch64-apple-darwin`:

```sh
cargo build --release --locked --target TARGET -p russet
cargo xtask package --target TARGET --bin-dir target/TARGET/release
```

`cargo xtask package` verifies each executable's architecture and writes
`dist/russet-development-TARGET.tar.gz`, or a `.zip` file for Windows
targets. The archive contains the CLI, license notices, documentation, the
installer, and, on macOS, the launchd plists from
[`distribution/launchd/`](distribution/launchd/), which run `russet --server`
and `russet --installd`. It contains no Python runtime. Packaging alone doesn't
install or activate services.

The installers keep previous installations in rollback generations; see
[`distribution/INSTALL.md`](distribution/INSTALL.md). No recipe or preference
format conversion is performed.
