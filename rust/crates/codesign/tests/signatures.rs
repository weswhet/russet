//! Package signature forms that older and third-party signers write and
//! `pkgutil --check-signature` accepts.

use russet_codesign::cms::verify_detached;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

#[test]
fn signature_without_signed_attributes_covers_the_content() {
    let signature = fixture("cms-no-signed-attributes.der");
    let content = fixture("cms-no-signed-attributes.content");
    let verified = verify_detached(&signature, &content).unwrap();
    assert_eq!(
        verified.signer.common_name(),
        "Russet test CMS signer",
        "{verified:?}"
    );
    assert!(verified.signed_attributes.is_empty());
    assert!(verified.timestamp.is_none() && verified.signing_time.is_none());
}

/// The package's `RSA` signature passes when it signs the checksum, or the
/// checksum's digest; the test signer then fails chain validation, which
/// comes after.
#[test]
fn package_rsa_signature_over_the_checksum_or_its_digest() {
    use russet_codesign::package::{verify, SignedPackage};
    let certificates = [fixture("package-rsa-signer.der")];
    let checksum = fixture("package-rsa.checksum");
    let mut other = checksum.clone();
    other[0] ^= 1;
    for name in ["package-rsa-direct.sig", "package-rsa-hashed.sig"] {
        let signature = fixture(name);
        let check = |checksum: &[u8]| {
            verify(
                &SignedPackage {
                    checksum_algorithm: russet_codesign::package::checksum_algorithm("sha1")
                        .unwrap(),
                    checksum,
                    rsa_signature: &signature,
                    rsa_certificates: &certificates,
                    cms_signature: None,
                },
                std::time::SystemTime::now(),
            )
            .unwrap_err()
        };
        let error = check(&checksum);
        assert!(
            !error.contains("package signature is invalid"),
            "{name}: {error}"
        );
        let error = check(&other);
        assert!(
            error.contains("package signature is invalid"),
            "{name}: {error}"
        );
    }
}

#[test]
fn signature_without_signed_attributes_rejects_other_content() {
    let signature = fixture("cms-no-signed-attributes.der");
    let mut content = fixture("cms-no-signed-attributes.content");
    content[0] ^= 1;
    let error = verify_detached(&signature, &content).unwrap_err();
    assert!(error.contains("Signature doesn't verify"), "{error}");
}
