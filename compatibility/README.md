# Compatibility contract

This folder holds the frozen contract that Russet builds against: data captured
from Python AutoPkg at commit `c36e58f8d3d8ddb70b6c2d848d2ceca7f767ce5c` and from
Munki `7.2.0.5787`. Russet's code and tests read these files. Russet contains no
Python and doesn't run Python AutoPkg.

The suites that capture these files and compare Russet with Python AutoPkg live
in [russet-compat](https://github.com/weswhet/russet-compat), together with the
records of their results. To change a captured file, run the matching capture
script there against your Russet checkout and include the regenerated file in
your Russet change.

## Files

| File | Contents | Read by |
| --- | --- | --- |
| `reference.json` | All 46 built-in processor manifests, and the command table, aliases, and declared options of the command-line interface | Processor registry and CLI tests |
| `cli-parser-reference.json` | Python AutoPkg's command-line parsers | CLI option parsing |
| `cli-processor-order.json` | The order in which `list-processors` shows the built-in processors | Processor registry |
| `community-processors.json` | Manifests, defaults, display order, source hashes, and the alias for the 12 community processors that Russet implements | Processor registry |
| `munki-makepkginfo-options.json` | The 46 `makepkginfo` option declarations from Munki 7.2.0 | Munki metadata |
| `chocolatey-render-reference.json` | Expected Chocolatey package rendering | Chocolatey tests |
| `predicate-fixtures.json` and `predicate-reference-results.json` | 22 predicate cases and their results from macOS NSPredicate | Predicate tests on every platform |
| `portable-fixtures.json` and `portable-reference-results.json` | 111 portable processor cases and frozen Python results | russet-compat |
| `cli-fixtures.json` | Command-line cases | russet-compat |
| `munki-fixtures.json` | 142 Munki processor cases | russet-compat |

## Manifest representation

`processors` maps recipe processor names to `description`, `input_variables`,
`output_variables`, and `lifecycle`. Inherited fields are resolved. Missing
manifest fields are represented as empty dictionaries (description as null).
Dictionary unpacking and URLDownloaderPython's copied and modified manifest are
resolved without executing source code. Source file SHA-256 hashes identify the
captured files.

Platform-dependent defaults retain a `python_expression` marker for
`signtool_default_path()` and `_default_use_python_native_extractor()`. The
Chocolatey `DefaultValue` sentinel retains a `python_sentinel` marker. These
markers describe reference behavior; they must never become recipe environment
values. CLI keyword arguments that require runtime evaluation use the same
`python_expression` representation. Options are grouped by their declaring
function; shared option helpers must be applied by callers.

This is a static API baseline, not an execution compatibility certification.
Defaults implemented only inside `main`, filesystem effects, exit status,
report contents, preference precedence, and subprocess arguments need the
runtime comparisons in [russet-compat](https://github.com/weswhet/russet-compat). `portable-reference-results.json` records a limited set of actual runtime
outcomes; it does not cover those complete release requirements.

## Target capability matrix

This matrix records target capabilities and an earlier representative corpus
checkpoint; counts are not totals for the expanded suites in russet-compat.
Failure counts include required-input checks; they do not establish every error
path. A successful disabled signature check does not verify a Windows signature.
Every processor must remain discoverable on every platform.
Portable paths of conditional processors must work without DMG support. macOS
native tooling is required only when a request actually needs Apple inspection,
mounting, packaging, installation, or signature checks. Metadata-only Munki
operations are portable. Predicate evaluation uses NSPredicate on macOS and the
documented subset on Windows and Linux.

| Processor | Target capability | Comparison coverage |
| --- | --- | --- |
| AppDmgVersioner | macOS native tools | 1 corpus failure (1 required-input); native app image metadata |
| AppPkgCreator | macOS native tools | pinned Python/Rust installed VM differential passed |
| ChocolateyPackager | Windows native tools | rendering and required-input fixtures; native Windows packing and active-default Python comparisons passed |
| CodeSignatureVerifier | macOS native tools | 1 corpus success; 2 corpus failure (1 required-input) |
| Copier | portable with conditional macOS operations | 7 corpus success; 2 corpus failure (1 required-input) |
| DeprecationWarning | portable | 1 corpus success |
| DmgCreator | macOS native tools | 1 corpus failure (1 required-input); native image creation with defaults |
| DmgMounter | macOS native tools | 1 corpus failure (0 required-input); abstract class: direct execution fails in both implementations |
| EndOfCheckPhase | portable | 1 corpus success |
| FileCreator | portable | 2 corpus success; 4 corpus failure (2 required-input) |
| FileFinder | portable with conditional macOS operations | 2 corpus success; 3 corpus failure (1 required-input) |
| FileMover | portable | 2 corpus success; 2 corpus failure (1 required-input) |
| FindAndReplace | portable | 3 corpus success; 1 corpus failure (1 required-input) |
| FlatPkgPacker | macOS native tools | 1 corpus failure (1 required-input); native packing |
| FlatPkgUnpacker | macOS native tools | 1 corpus failure (1 required-input); native unpacking with and without payload |
| GitHubReleasesInfoProvider | portable | 1 corpus failure (1 required-input); 12 HTTP cases |
| InstallFromDMG | macOS native tools | 1 corpus failure (1 required-input); pinned Python/Rust installed VM differential passed |
| Installer | macOS native tools | 1 corpus failure (1 required-input); pinned Python/Rust installed VM differential passed |
| MunkiCatalogBuilder | portable | 1 corpus success |
| MunkiImporter | portable with conditional macOS operations | 1 corpus failure (1 required-input); native metadata/import tests; see Munki entrypoint harness |
| MunkiInfoCreator | portable with conditional macOS operations | 1 corpus failure (1 required-input); native metadata/import tests; see Munki entrypoint harness |
| MunkiInstallsItemsCreator | portable with conditional macOS operations | 1 corpus failure (1 required-input); native metadata/import tests; see Munki entrypoint harness |
| MunkiOptionalReceiptEditor | portable | 1 corpus success; 1 corpus failure (1 required-input) |
| MunkiPkginfoMerger | portable | 1 corpus success; 1 corpus failure (1 required-input) |
| MunkiSetDefaultCatalog | portable | 1 corpus success; positive native preference read in disposable VM |
| PackageRequired | portable | 1 corpus success; 1 corpus failure (0 required-input) |
| PathDeleter | portable | 4 corpus success; 2 corpus failure (1 required-input) |
| PkgCopier | portable with conditional macOS operations | 3 corpus success; 2 corpus failure (1 required-input) |
| PkgCreator | macOS native tools | 1 corpus failure (1 required-input); pinned Python/Rust installed VM differential passed |
| PkgExtractor | macOS native tools | 1 corpus failure (1 required-input); native bundle extraction |
| PkgInfoCreator | portable | 2 corpus success; 2 corpus failure (1 required-input) |
| PkgPayloadUnpacker | macOS native tools | 1 corpus failure (1 required-input); native payload extraction |
| PkgRootCreator | portable | 1 corpus success; 1 corpus failure (1 required-input) |
| PlistEditor | portable | 2 corpus success; 1 corpus failure (1 required-input) |
| PlistReader | portable with conditional macOS operations | 2 corpus success; 2 corpus failure (1 required-input) |
| SignToolVerifier | Windows native tools | required-input and disabled-verification fixtures; native signed/unsigned and active-default Python comparisons passed |
| SparkleUpdateInfoProvider | portable | 1 corpus failure (1 required-input); 5 HTTP cases |
| StopProcessingIf | portable | 2 corpus success; 2 corpus failure (1 required-input) |
| Symlinker | portable | 3 corpus success; 2 corpus failure (1 required-input) |
| URLDownloader | portable | 1 corpus failure (1 required-input); 14 HTTP cases |
| URLDownloaderPython | portable | 1 corpus failure (1 required-input); 14 HTTP cases |
| URLGetter | portable | 1 corpus failure (0 required-input); abstract class: direct execution fails in both implementations |
| URLTextSearcher | portable | 1 corpus failure (1 required-input); 4 HTTP cases |
| Unarchiver | portable with conditional macOS operations | 4 corpus success; 2 corpus failure (0 required-input) |
| VariableSetter | portable | 1 corpus success |
| Versioner | portable with conditional macOS operations | 5 corpus success; 3 corpus failure (1 required-input) |

## Default and required-input audit

All 46 static manifests are checked against the pinned source. The processor
corpus includes required-input rejection for each of the 36 manifests declaring
required inputs. This checks the preflight boundary, not every runtime failure.
The table below covers every manifest with declared defaults. Omission exercises
a default but does not prove all branches controlled by that value.

| Processor | Declared defaults | Execution evidence and limits |
| --- | --- | --- |
| AppPkgCreator | `force_pkg_build`, `pkgbuild_args`, `version_key` | All three omitted in the pinned Python/Rust installed VM app-package differential, including repeat execution. |
| ChocolateyPackager | `KEEP_BUILD_DIRECTORY`, `chocoexe_path`, `installer_checksum_type`, `installer_path` | Configuration rendering, real Windows packaging, and active-default Python comparisons passed. |
| CodeSignatureVerifier | `deep_verification`, `strict_verification` | Both omitted in the real Apple signature success and rejection fixtures. |
| DmgCreator | `dmg_filesystem`, `dmg_format`, `dmg_zlib_level` | All three omitted in the native image-creation differential. |
| FileFinder | `find_method` | Omitted in portable file-finding cases. |
| FindAndReplace | `result_output_var_name` | Omitted in portable replacement cases. |
| FlatPkgUnpacker | `skip_payload` | False default and explicit true branch both pass native differential cases. |
| GitHubReleasesInfoProvider | `CURL_PATH`, `GITHUB_RELEASES_PER_PAGE`, `GITHUB_TOKEN_PATH`, `GITHUB_URL` | Default curl executable and page size exercised by HTTP fixtures; API URL and token path overridden for isolation. |
| MunkiImporter | `MUNKILIB_DIR`, `MUNKI_PKGINFO_FILE_EXTENSION`, `MUNKI_REPO_PLUGIN`, `force_munki_repo_lib` | Native FileRepo and metadata option fixtures; entrypoint results are in the russet-compat evidence records. |
| MunkiOptionalReceiptEditor | `MUNKILIB_DIR`, `MUNKI_REPO_PLUGIN`, `force_munki_repo_lib` | All three omitted in the receipt-edit differential. |
| PathDeleter | `continue_on_error` | Omitted in successful and missing-path failure cases. |
| PkgCreator | `force_pkg_build`, `pkgbuild_args` | Both omitted in the pinned Python/Rust installed VM differential. |
| PlistReader | `plist_keys` | Omitted in portable plist-reading cases. |
| SignToolVerifier | `additional_arguments`, `signtool_path` | Real Windows signed/unsigned checks and active-default Python comparisons passed, including omitted and explicit tool paths. |
| SparkleUpdateInfoProvider | `alternate_xmlns_url`, `urlencode_path_component` | Both omitted in local appcast HTTP cases. |
| URLDownloader | `CHECK_FILESIZE_ONLY`, `COMPUTE_HASHES`, `DOWNLOAD_MISSING_FILE`, `HEADERS_TO_TEST`, `prefetch_filename` | All five omitted in initial download and conditional cache cases. |
| URLDownloaderPython | `CHECK_FILESIZE_ONLY`, `COMPUTE_HASHES`, `DOWNLOAD_MISSING_FILE`, `HEADERS_TO_TEST`, `prefetch_filename` | All five omitted in initial download and conditional cache cases. |
| URLTextSearcher | `result_output_var_name` | Omitted in named-capture HTTP cases. |
| Unarchiver | `USE_PYTHON_NATIVE_EXTRACTOR` | Platform-selected default exercised by the default ZIP extraction case. |
| Versioner | `plist_version_key`, `skip_single_root_dir` | Both omitted in plist and ZIP cases; explicit single-root case also covered. |

URLTextSearcher supports global ASCII flags and mixed nested ASCII/Unicode scopes,
including classes, ranges, boundaries, case folding, backreferences, and capture
text preservation. Case-insensitive backreferences use the mode at the reference
site and CPython 3.11.9 simple lowercase values, with candidate-scalar UTF-8
advancement. The pinned fancy-regex compatibility patch is opt-in and carries
its upstream, Python, and Unicode notices into distribution archives.
ASCII matching operates on the original Unicode text without a distinct-scalar
mapping limit. Tests cover the entire Unicode scalar alphabet and five HTTP
comparisons above the former 137,468-scalar limit. Unicode IGNORECASE includes
Python's Turkish I, Kelvin sign, and long s behavior for literal and
character-class matching. The expanded HTTP corpus passes 100 cases against
the published Python runtime. Native Windows signing and Chocolatey comparisons
also pass. The Munki option corpus covers all captured declarations and aliases;
these finite corpora do not establish parity for every possible input.

## Release boundaries

- Reject custom Python processors and non-FileRepo Munki backends before steps run.
- Reject `force_munki_repo_lib=True`; do not load Python Munki libraries.
- Keep `URLDownloaderPython` as a Rust compatibility name.
- Keep MunkiCatalogBuilder's warning and no-op behavior; catalog rebuilding is excluded.
- Reject unsupported platform operations explicitly.
- Portable predicates include comparisons, boolean operators, parentheses,
  literals, environment key paths, membership, and string matching with case and
  diacritic modifiers. Reject syntax outside this subset.
- Munki metadata parity is pinned to `7.2.0.5787`. Its source-backed makepkginfo option
  declarations are frozen in `munki-makepkginfo-options.json`. Native fixtures
  cover each declaration and alias; complete behavior and failure-path coverage
  remains a release gate. Unknown options must fail.
- CI targets are macOS arm64 and x86-64, Windows x86-64, and Linux x86-64. Older
  operating systems have no initial compatibility promise.
- Russet ships no plugin API, bundled Python, or Python bridge.

The russet-compat suites give each implementation its own cache and its own
mutable FileRepo repository, and they normalize only documented
nondeterministic fields.

## Native predicate fixtures

`predicate-fixtures.json` contains 22 cases captured from the real reference
NSPredicate on macOS with Python 3.11.9. Cases include nested paths, null,
boolean and numeric equality, integers above the exact-double range, collection
membership, string modifiers, wildcard matching, and invalid syntax.
`predicate-reference-results.json` freezes the results with the pinned source
commit, capture platform, and a digest of the fixture definitions.

The portable Rust evaluator is tested directly against the captured results,
including on macOS, where normal processor execution uses Foundation. The CLI
test `rust/crates/cli/tests/predicate_reference.rs` also runs all 22 cases
through `autopkg-rs processor-run` on every platform. It compares the final
environment, exit status, and files with the frozen results, and rejects
fixtures whose digest doesn't match. To capture the native results again, use
the predicate suite in russet-compat on macOS.

## Munki metadata option reference

The [official v7.2.0 release](https://github.com/munki/munki/releases/tag/v7.2.0)
identifies the core and admin build as `7.2.0.5787`. Its exact commit is
`8896fe831e870732aac760f76566762fc35d5d00`. This release's makepkginfo command is
implemented in Swift. The capture uses its
[option groups](https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/shared/admin/pkginfoOptions.swift)
and [command declaration](https://github.com/munki/munki/blob/8896fe831e870732aac760f76566762fc35d5d00/code/cli/munki/makepkginfo/makepkginfo.swift).

`munki-makepkginfo-options.json` records 46 option declarations, aliases, enum
values, the optional installer-item argument, and source hashes. It includes
private print-warning switches and distinguishes implicit help options. Each
option retains its Swift property declaration to preserve type and default
information. russet-compat's `Scripts/capture_munki_options.py` regenerates it,
or verifies it with `--check`; the script downloads those exact pinned source
files and never executes them.

The static inventory alone doesn't verify metadata behavior. Native fixtures
cover every captured option declaration and alias, scripts, receipts, flat and
bundle packages, images, staged installer metadata, icons, and repeated imports.
The russet-compat Munki suite runs 174 entrypoint cases, 142 ordinary cases and
32 logging cases at verbosity levels 0, 1, and 2, against the pinned Munki tools.
Its [option coverage](https://github.com/weswhet/russet-compat/blob/main/docs/munki-option-coverage.md)
maps each declaration to its fixtures.
