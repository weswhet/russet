//! Installer package signatures, checked the way
//! `pkgutil --check-signature` checks them.
//!
//! A signed flat package carries an `RSA` signature over its TOC checksum
//! (or, from some signers, over the checksum's digest),
//! with the signing chain, and usually a CMS signature over the same
//! checksum that holds a trusted timestamp. The signature is valid when the
//! RSA signature verifies with the leaf certificate, the CMS signature (if
//! present) verifies with the same certificate, and the chain reaches an
//! Apple root with every certificate valid at the timestamp, or at the
//! current time when there's no timestamp.

use crate::trust::{self, oid, Cert, Chain, Purpose};
use der::asn1::ObjectIdentifier;
use std::time::SystemTime;

/// The parts of a signed package that the signature covers.
pub struct SignedPackage<'a> {
    /// The TOC checksum algorithm, such as SHA-1.
    pub checksum_algorithm: ObjectIdentifier,
    /// The verified TOC checksum.
    pub checksum: &'a [u8],
    /// The `RSA` signature bytes.
    pub rsa_signature: &'a [u8],
    /// The certificates listed with the `RSA` signature, leaf first.
    pub rsa_certificates: &'a [Vec<u8>],
    /// The `CMS` signature bytes, if present.
    pub cms_signature: Option<&'a [u8]>,
}

/// A valid package signature.
#[derive(Clone, Debug)]
pub struct PackageSignature {
    /// The `Status` line `pkgutil` prints.
    pub status: &'static str,
    /// When the trusted timestamp says the package was signed.
    pub timestamp: Option<SystemTime>,
    /// The validated chain, leaf first.
    pub chain: Chain,
}

/// Maps a checksum style such as `sha1` to its digest algorithm.
pub fn checksum_algorithm(style: &str) -> Option<ObjectIdentifier> {
    match style.to_ascii_lowercase().as_str() {
        "sha1" => Some(oid::SHA1),
        "sha256" => Some(oid::SHA256),
        "sha512" => Some(oid::SHA512),
        _ => None,
    }
}

/// Verifies a package signature at time `now`.
pub fn verify(package: &SignedPackage, now: SystemTime) -> Result<PackageSignature, String> {
    let mut certificates = package
        .rsa_certificates
        .iter()
        .map(|der| Cert::from_der(der))
        .collect::<Result<Vec<_>, _>>()?;
    if certificates.is_empty() {
        return Err("The package signature has no certificates".into());
    }
    let leaf = certificates.remove(0);
    // Most signers sign the checksum as a digest. Some sign it as a message,
    // so the signature holds the digest of the checksum; `pkgutil` accepts
    // both, and either way the signature covers this checksum.
    let algorithm = &package.checksum_algorithm;
    let signed = trust::digest_info(algorithm, package.checksum)?;
    trust::verify_raw_pkcs1(&leaf, &signed, package.rsa_signature)
        .or_else(|error| {
            let hashed = trust::digest(algorithm, package.checksum)?;
            let signed = trust::digest_info(algorithm, &hashed)?;
            trust::verify_raw_pkcs1(&leaf, &signed, package.rsa_signature).map_err(|_| error)
        })
        .map_err(|e| format!("The package signature is invalid: {e}"))?;
    let mut timestamp = None;
    if let Some(bytes) = package.cms_signature {
        let cms = crate::cms::verify_detached(bytes, package.checksum)
            .map_err(|e| format!("The package's CMS signature is invalid: {e}"))?;
        if cms.signer.der != leaf.der {
            return Err("The package's two signatures name different signers".into());
        }
        timestamp = cms.timestamp;
        certificates.extend(cms.certificates.into_iter().filter(|c| c.der != leaf.der));
    }
    let chain = trust::validate(
        &leaf,
        &certificates,
        timestamp.unwrap_or(now),
        Purpose::PackageSigning,
    )?;
    let status = if chain.leaf().has_extension(&oid::DEVELOPER_ID_INSTALLER) {
        "signed by a developer certificate issued by Apple for distribution"
    } else {
        "signed by a certificate issued by Apple"
    };
    Ok(PackageSignature {
        status,
        timestamp,
        chain,
    })
}

/// Formats a time like `pkgutil`: `2026-09-08 16:21:28 +0000`.
pub fn format_time(time: SystemTime) -> String {
    let seconds = trust::unix_seconds(time);
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Civil-from-days, after Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} +0000",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

impl PackageSignature {
    /// Lines in the layout of `pkgutil --check-signature`, without the
    /// notarization line, which needs Apple's service.
    pub fn report(&self, name: &str) -> Vec<String> {
        let mut lines = vec![
            format!("Package \"{name}\":"),
            format!("   Status: {}", self.status),
        ];
        if let Some(time) = self.timestamp {
            lines.push(format!(
                "   Signed with a trusted timestamp on: {}",
                format_time(time)
            ));
        }
        lines.push("   Certificate Chain:".into());
        for (index, cert) in self.chain.certificates.iter().enumerate() {
            lines.push(format!("    {}. {}", index + 1, cert.common_name()));
            lines.push(format!("       Expires: {}", format_time(cert.not_after())));
            let hex: Vec<String> = cert.sha256().iter().map(|b| format!("{b:02X}")).collect();
            lines.push("       SHA256 Fingerprint:".into());
            for chunk in hex.chunks(22) {
                lines.push(format!("           {}", chunk.join(" ")));
            }
        }
        lines
    }
}
