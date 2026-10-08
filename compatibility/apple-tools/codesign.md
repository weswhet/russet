# codesign --verify

Russet's `russet-codesign` crate reproduces
`codesign --verify [--deep] [--strict] --test-requirement=REQUIREMENT` on
Linux. These notes record the behavior it matches. They were checked against
`codesign` on macOS 27.0 (build 26A428) with 19 installed third-party apps
and with tampered copies of a signed app. The tests in
`rust/crates/codesign` repeat the comparison with a committed Developer ID
signed bundle on every platform.

## What's verified

- **Executable.** Every architecture of the bundle's executable
  (`CFBundleExecutable`, or the bundle's name when that key is missing). For
  each architecture, every code directory (SHA-1 and SHA-256) must match the
  code pages up to its code limit, and its special slots must match the
  Info.plist (or the binary's `__TEXT,__info_plist` section for a standalone
  executable), the requirements blob, the resource seal, and the
  entitlements.
- **Signature.** The CMS signature must cover the primary code directory, and
  its CDHashes attribute must list every code directory. Apple encodes it in
  BER, which Russet converts to DER before parsing. A trusted timestamp
  (RFC 3161, from Apple's timestamp authority) is verified, and the chain is
  checked at its time.
- **Chain.** The leaf must allow code signing and chain to Apple Root CA, G2,
  or G3. Unknown critical extensions fail.
- **Resource seal.** Every file under the bundle's content folder is matched
  against `rules2` by weight. Omitted files are skipped; sealed files must
  match their SHA-256 (`hash2`); symlinks must match their recorded target;
  nested code (bundles and Mach-O files under a `nested` rule) must match its
  recorded CDHash and satisfy its recorded requirement. Files that aren't
  sealed, and sealed files that are missing and not `optional`, fail.
  `_CodeSignature`, the legacy top-level `CodeResources` link, and the
  executable aren't resources.
- **`--deep`.** Nested bundles are verified completely, including their own
  resource seals.
- **`--strict`.** Anything in the bundle with a resource fork or Finder info
  fails with "resource fork, Finder information, or similar detritus not
  allowed", and symlinks may not point outside the bundle. Without
  `strict_verification`, or with it set to `false`, these checks are off, as
  they are for `codesign` without `--strict`.

## Requirements

Supported clauses: `identifier`, `anchor apple generic` (the chain ends at an
Apple root), `certificate leaf|root|N[subject.CN|O|OU|C|L|ST|email]` with
`= value` (wildcards at either end) or existence, `certificate N[field.OID]`
existence, `cdhash`, `always`, `never`, `and`, `or`, `not`, parentheses, and
comments. All 28 requirements in the core AutoPkg recipes use only these.

Anything else, such as `anchor apple`, `notarized`, `info[...]`,
`entitlement[...]`, or `certificate ... trusted`, makes verification fail with
"codesign rejected its arguments", so the native verifier never treats an
unknown clause as satisfied. `codesign` accepts some of these, so this is a
known difference.

## Not checked

Notarization, stapled tickets, revocation (OCSP and CRLs), and Gatekeeper
policy. Signatures from before macOS 10.9 (without `rules2`) and
resource-only bundles aren't supported.

## Requirement language

Observed with `codesign --verify -R` on macOS 27.0:

- Negation is `!`. `codesign` rejects `not` as a syntax error (exit 1), so the
  native parser rejects it too.
- `anchor apple` is satisfied by code Apple signs itself, such as
  `/usr/bin/true`: the chain ends at Apple Root CA, and the certificate
  directly below it is "Apple Code Signing Certification Authority" from
  "Apple Inc.". Developer ID code doesn't satisfy it. The native verifier
  checks exactly that, and `rust/crates/codesign/tests/apple_requirements.rs`
  compares its decisions with `codesign` on macOS CI.
