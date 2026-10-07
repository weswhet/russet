# pkgutil

Russet's `russet-pkgutil` crate reproduces `pkgutil --expand`,
`pkgutil --flatten`, and `pkgutil --check-signature` on Linux. These notes
record the behavior it matches, observed with `pkgutil` on macOS 27.0 (build
26A428). The tests in `rust/crates/pkgutil` repeat the comparisons on every
macOS CI runner.

## --expand

- The destination must not exist ("Error 17: File exists"), and its parent
  must exist ("Error 2: No such file or directory").
- The xar archive is extracted with the modes in its table of contents, so a
  component folder in a product archive is `0700`, as `productbuild` records
  it.
- Each component's `Scripts` archive becomes a folder holding the archive's
  contents. `._name` AppleDouble members stay ordinary files; they aren't
  merged into extended attributes.
- `Payload` stays a compressed archive.

## --flatten

- An existing destination is replaced.
- `Bom`, `PackageInfo`, and `Distribution` are stored with bzip2
  compression; other files, including `Payload`, are stored as is.
- `Scripts` and `Payload` folders are stored as gzip-compressed odc cpio
  archives.
- Output is compared by expanding it again, not byte for byte.

## --check-signature

A signed package carries two signatures over its table-of-contents checksum:

- `<signature style="RSA">`: a PKCS #1 v1.5 signature of the DER
  `DigestInfo` for the checksum (SHA-1 in the packages examined), with the
  certificate chain, leaf first, in `KeyInfo`.
- `<x-signature style="CMS">`: a detached CMS signature, BER-encoded with
  indefinite lengths and zero padding, whose signed attributes carry the
  checksum's digest. Its unsigned attributes hold an RFC 3161 timestamp from
  Apple's timestamp authority.

`pkgutil` reports the package as signed when both verify with the same leaf
certificate and the chain reaches Apple Root CA. With a trusted timestamp, the
chain is checked at the timestamp's time, so a package stays valid after its
certificate expires. The certificate chain it prints, leaf first, is what
`CodeSignatureVerifier` compares with `expected_authority_names`.

Russet's replacement trusts only Apple Root CA, G2, and G3, and requires the
leaf to allow package signing or code signing. It doesn't check notarization
or revocation, so it doesn't print `pkgutil`'s `Notarization` line.
