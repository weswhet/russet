# Test fixtures

`MSCDockTilePlugin.docktileplugin` is copied unmodified from Managed Software
Center in Munki (<https://github.com/munki/munki>), which is licensed under the
Apache License 2.0. It's signed by "Developer ID Application: Mac Admins Open
Source (T4SK8ZXCXG)" with a trusted timestamp, so the tests check a real
Developer ID code signature on every platform.

`cms-no-signed-attributes.der` is a detached CMS signature over
`cms-no-signed-attributes.content` with no signed attributes, the form some
older package signers use. It was made with `openssl cms -sign -binary
-noattr -md sha256` and a throwaway self-signed key for "Russet test CMS
signer".

`package-rsa-direct.sig` and `package-rsa-hashed.sig` are PKCS #1 v1.5 SHA-1
signatures by the same throwaway key, whose certificate is
`package-rsa-signer.der`. The first signs `package-rsa.checksum` as a digest,
like most package signers; the second signs the checksum's SHA-1 digest, like
the signers of some Developer ID packages. The private key isn't kept.
