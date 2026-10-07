# Native primary-repository processors

The 94 processor-related rejections in the [initial live sweep](https://github.com/weswhet/russet-compat/blob/main/evidence/live-recipes-2026-10-07.md)
use 12 custom processor classes and one shared-recipe alias. Russet ports these
classes to Rust and registers them as built-ins. The original 46 manifests
remain frozen; `community-processors.json` adds the new manifests, defaults,
display ordering, source hashes, and alias. The CLI exposes 59 names for 58
implementations.

| Native processor | Work covered |
|---|---|
| AdobeAcrobatProUpdateInfoProvider | Acrobat updater metadata |
| AdobeFlashURLProvider | Flash update XML |
| AdobeReaderURLProvider | Reader download discovery |
| AdobeReaderRepackager | Reader XI/DC package repackaging |
| AutoPkgSourceFinder | Expanded AutoPkg source discovery |
| GenerateRelocatablePython | Relocatable Python framework artifact |
| BarebonesURLProvider | Bare Bones product metadata |
| MSOfficeMacURLandUpdateInfoProvider | Office/Edge metadata and deprecated-product handling |
| MozillaURLProvider | Mozilla product, platform, and locale selection |
| MakeCatalogsProcessor | Native FileRepo catalogs and icon hashes |
| PuppetlabsProductsURLProvider | Puppet product download discovery |
| SassafrasK2ClientCustomizer | Native K2 client package customization |

`com.github.autopkg.AutoPkgGitMaster/GenerateRelocatablePython` resolves to the
compiled `GenerateRelocatablePython` implementation. Arbitrary Python processors
remain unsupported and are rejected during recipe validation.

## Reference and boundaries

The processor sources come from [autopkg/recipes at
`0f7b61ab061c77710be4140c404910a11f89fc7b`](https://github.com/autopkg/recipes/tree/0f7b61ab061c77710be4140c404910a11f89fc7b).
[russet-compat](https://github.com/weswhet/russet-compat) keeps those pinned
sources, with their Apache-2.0 copyright notices and Adobe package scripts, in
`community-source/`. Its `Scripts/capture_community_processors.py --check`
verifies all 12 captured contracts against those sources without importing
Python modules.

`GenerateRelocatablePython` ports the framework-building behavior of Greg Neagle's
[relocatable-python at `8ee72fe3a5dbef733365370ebf44f25022b895ef`](https://github.com/gregneagle/relocatable-python/tree/8ee72fe3a5dbef733365370ebf44f25022b895ef),
licensed under Apache-2.0. Other builder revisions are explicitly rejected. It
requires macOS and native Apple tools. It runs the Python executable produced
inside its output artifact to install requirements and verify HTTPS; it does not
require a host Python interpreter or execute the upstream Python builder.

`MakeCatalogsProcessor` implements the FileRepo behavior of Munki's Apache-2.0
[makecatalogs at v7.2.0](https://github.com/munki/munki/tree/v7.2.0).
This is a catalog rebuild operation, distinct from the original deprecated
`MunkiCatalogBuilder`, which remains a warning and no-op. Non-FileRepo backends
and Python Munki libraries remain unsupported.

macOS is required for Reader repackaging and framework building. Portable
metadata operations remain available on other platforms. Historical providers
can fail when vendors retire their endpoints; implementation coverage does not
imply those upstream services still work. Office 2016 recipes deliberately stop
with a deprecation warning, matching the pinned source.

The pinned `RelocatablePython` recipes have a nested default requirements path
containing `%RECIPE_CACHE_DIR%`. Python AutoPkg's single-pass input substitution
leaves that token unresolved; Rust preserves this behavior. Supply a concrete
`REQUIREMENTS_PATH` when using those recipes. The hosted artifact test uses an
absolute requirements path and verifies a successful build and relocation.

## Validation

The comparison suites run in russet-compat:

- `differential_community_modern.py`: 62 controlled HTTP cases for Office,
  Mozilla, and Bare Bones; compares environments, statuses, logs, errors, and requests.
- `differential_community_legacy.py`: 41 cases for Adobe, Puppet, and Sassafras,
  including successful logs, native package/DMG repackaging, and mount cleanup. Source failures
  preserve the distinction between processor errors (exit 10) and uncaught
  reference exceptions (exit 1).
- `differential_community_builders.py`: source discovery and native Munki
  FileRepo comparisons; hosted macOS additionally builds a framework and checks
  HTTPS and pip after moving it to a different path.

Russet's engine tests cover deprecation reports, stop behavior, registry
expansion, and existing override trust records for promoted processors.

Controlled comparisons use independent temporary directories. Live validation
uses the same pinned 94-recipe list on GitHub-hosted macOS runners. The
[94-recipe follow-up results](https://github.com/weswhet/russet-compat/blob/main/evidence/community-live-recipes-2026-10-07.md) record 81 successful
exits (65 completed workflows, 15 deprecation stops, and one catalog skip),
12 failures, and one timeout, with no processor rejections. Focused Windows and
requirements-path retries are recorded separately from the historical baseline.
No local VM is required.
