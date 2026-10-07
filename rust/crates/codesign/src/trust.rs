//! Certificate chains anchored at Apple's roots, and the signature checks
//! they rely on.

use der::asn1::{ObjectIdentifier, UintRef};
use der::{Decode, Encode, Sequence};
use sha2::Digest;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use x509_cert::Certificate;

/// The roots Russet trusts. See `data/README.md`.
const APPLE_ROOTS: [&[u8]; 3] = [
    include_bytes!("../data/AppleIncRootCertificate.cer"),
    include_bytes!("../data/AppleRootCA-G2.cer"),
    include_bytes!("../data/AppleRootCA-G3.cer"),
];
const MAX_CHAIN: usize = 6;

pub mod oid {
    use der::asn1::ObjectIdentifier as Oid;
    pub const COMMON_NAME: Oid = Oid::new_unwrap("2.5.4.3");
    pub const RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
    pub const SHA1_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
    pub const SHA256_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
    pub const SHA384_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
    pub const SHA512_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
    pub const ECDSA_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
    pub const ECDSA_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
    pub const EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
    pub const P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
    pub const P384: Oid = Oid::new_unwrap("1.3.132.0.34");
    pub const SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
    pub const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
    pub const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
    pub const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");
    pub const BASIC_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.19");
    pub const KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.15");
    pub const EXTENDED_KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.37");
    pub const CERTIFICATE_POLICIES: Oid = Oid::new_unwrap("2.5.29.32");
    pub const CODE_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.3");
    pub const TIME_STAMPING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.8");
    /// Apple's package-signing extended key usage.
    pub const APPLE_PACKAGE_SIGNING: Oid = Oid::new_unwrap("1.2.840.113635.100.4.13");
    /// Marks a Developer ID Installer leaf certificate.
    pub const DEVELOPER_ID_INSTALLER: Oid = Oid::new_unwrap("1.2.840.113635.100.6.1.14");
    /// Marks a Developer ID Application leaf certificate.
    pub const DEVELOPER_ID_APPLICATION: Oid = Oid::new_unwrap("1.2.840.113635.100.6.1.13");
}

/// Apple marks its certificate types with critical extensions under this
/// arc. A verifier must understand every critical extension, so these are
/// the only ones accepted besides the standard ones above.
const APPLE_ARC: &str = "1.2.840.113635.100.6.";

/// A parsed certificate and its original encoding.
#[derive(Clone, Debug)]
pub struct Cert {
    pub der: Vec<u8>,
    pub(crate) parsed: Certificate,
}

fn error(message: impl Into<String>) -> String {
    message.into()
}

impl Cert {
    pub fn from_der(der: &[u8]) -> Result<Self, String> {
        let parsed = Certificate::from_der(der).map_err(|e| format!("Invalid certificate: {e}"))?;
        Ok(Self {
            der: der.to_vec(),
            parsed,
        })
    }

    /// The subject's common name, as `pkgutil` and `codesign` print it.
    pub fn common_name(&self) -> String {
        name_cn(&self.parsed.tbs_certificate.subject).unwrap_or_default()
    }

    /// Every value of subject attribute `id`, such as the organizational
    /// unit, which holds the team ID.
    pub fn subject_values(&self, id: &ObjectIdentifier) -> Vec<String> {
        let mut values = Vec::new();
        for rdn in self.parsed.tbs_certificate.subject.0.iter() {
            for attribute in rdn.0.iter().filter(|a| a.oid == *id) {
                values.push(any_string(&attribute.value));
            }
        }
        values
    }

    pub fn sha256(&self) -> [u8; 32] {
        sha2::Sha256::digest(&self.der).into()
    }

    pub fn not_before(&self) -> SystemTime {
        UNIX_EPOCH
            + self
                .parsed
                .tbs_certificate
                .validity
                .not_before
                .to_unix_duration()
    }

    pub fn not_after(&self) -> SystemTime {
        UNIX_EPOCH
            + self
                .parsed
                .tbs_certificate
                .validity
                .not_after
                .to_unix_duration()
    }

    /// Whether the certificate carries extension `id`.
    pub fn has_extension(&self, id: &ObjectIdentifier) -> bool {
        self.extensions().any(|e| e.extn_id == *id)
    }

    fn extensions(&self) -> impl Iterator<Item = &x509_cert::ext::Extension> {
        self.parsed.tbs_certificate.extensions.iter().flatten()
    }

    /// Extended key usages, empty when the extension is absent.
    pub fn extended_key_usages(&self) -> Vec<ObjectIdentifier> {
        self.extensions()
            .find(|e| e.extn_id == oid::EXTENDED_KEY_USAGE)
            .and_then(|e| Vec::<ObjectIdentifier>::from_der(e.extn_value.as_bytes()).ok())
            .unwrap_or_default()
    }

    fn is_ca(&self) -> bool {
        #[derive(Sequence)]
        struct BasicConstraints {
            #[asn1(default = "Default::default")]
            ca: bool,
            path_len: Option<u32>,
        }
        self.extensions()
            .find(|e| e.extn_id == oid::BASIC_CONSTRAINTS)
            .and_then(|e| BasicConstraints::from_der(e.extn_value.as_bytes()).ok())
            .is_some_and(|b| b.ca)
    }

    fn check_critical_extensions(&self) -> Result<(), String> {
        for extension in self.extensions().filter(|e| e.critical) {
            let id = extension.extn_id;
            let known = [
                oid::BASIC_CONSTRAINTS,
                oid::KEY_USAGE,
                oid::EXTENDED_KEY_USAGE,
                oid::CERTIFICATE_POLICIES,
            ]
            .contains(&id)
                || id.to_string().starts_with(APPLE_ARC);
            if !known {
                return Err(format!(
                    "Certificate {:?} has an unsupported critical extension {id}",
                    self.common_name()
                ));
            }
        }
        Ok(())
    }

    /// Verifies that `issuer` signed this certificate.
    fn signed_by(&self, issuer: &Cert) -> Result<(), String> {
        if self.parsed.tbs_certificate.issuer != issuer.parsed.tbs_certificate.subject {
            return Err(error("issuer name doesn't match"));
        }
        let tbs = self
            .parsed
            .tbs_certificate
            .to_der()
            .map_err(|e| e.to_string())?;
        verify_message(
            issuer,
            &self.parsed.signature_algorithm.oid,
            None,
            &tbs,
            self.parsed.signature.raw_bytes(),
        )
    }
}

fn any_string(value: &der::Any) -> String {
    if let Ok(s) = value.decode_as::<der::asn1::Utf8StringRef>() {
        return s.to_string();
    }
    if let Ok(s) = value.decode_as::<der::asn1::PrintableStringRef>() {
        return s.to_string();
    }
    if let Ok(s) = value.decode_as::<der::asn1::Ia5StringRef>() {
        return s.to_string();
    }
    String::from_utf8_lossy(value.value()).into_owned()
}

fn name_cn(name: &x509_cert::name::Name) -> Option<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|a| a.oid == oid::COMMON_NAME)
        .map(|a| any_string(&a.value))
}

/// Verifies `signature` over `message` with `signer`'s public key.
/// `algorithm` is a signature algorithm, or `rsaEncryption` with the digest
/// in `digest` (as CMS signer infos record it).
pub(crate) fn verify_message(
    signer: &Cert,
    algorithm: &ObjectIdentifier,
    digest: Option<&ObjectIdentifier>,
    message: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    use ring::signature as sig;
    let spki = &signer.parsed.tbs_certificate.subject_public_key_info;
    let key = spki.subject_public_key.raw_bytes();
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.decode_as::<ObjectIdentifier>().ok());
    let rsa = spki.algorithm.oid == oid::RSA;
    let ec = spki.algorithm.oid == oid::EC_PUBLIC_KEY;
    let scheme: &dyn sig::VerificationAlgorithm = match (*algorithm, digest.copied()) {
        (oid::SHA256_RSA, _) | (oid::RSA, Some(oid::SHA256)) if rsa => {
            &sig::RSA_PKCS1_2048_8192_SHA256
        }
        (oid::SHA384_RSA, _) | (oid::RSA, Some(oid::SHA384)) if rsa => {
            &sig::RSA_PKCS1_2048_8192_SHA384
        }
        (oid::SHA512_RSA, _) | (oid::RSA, Some(oid::SHA512)) if rsa => {
            &sig::RSA_PKCS1_2048_8192_SHA512
        }
        (oid::SHA1_RSA, _) | (oid::RSA, Some(oid::SHA1)) if rsa => {
            &sig::RSA_PKCS1_1024_8192_SHA1_FOR_LEGACY_USE_ONLY
        }
        (oid::ECDSA_SHA256, _) if ec && curve == Some(oid::P256) => &sig::ECDSA_P256_SHA256_ASN1,
        (oid::ECDSA_SHA256, _) if ec && curve == Some(oid::P384) => &sig::ECDSA_P384_SHA256_ASN1,
        (oid::ECDSA_SHA384, _) if ec && curve == Some(oid::P256) => &sig::ECDSA_P256_SHA384_ASN1,
        (oid::ECDSA_SHA384, _) if ec && curve == Some(oid::P384) => &sig::ECDSA_P384_SHA384_ASN1,
        _ => return Err(format!("Unsupported signature algorithm {algorithm}")),
    };
    sig::UnparsedPublicKey::new(scheme, key)
        .verify(message, signature)
        .map_err(|_| error("Signature doesn't verify"))
}

/// Verifies an RSA PKCS #1 v1.5 signature over a value that was signed as
/// is, without hashing. xar's `RSA` signature signs the TOC checksum this
/// way, which `ring` can't check, so this does the public-key operation
/// directly.
pub(crate) fn verify_raw_pkcs1(
    signer: &Cert,
    signed: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    #[derive(Sequence)]
    struct RsaPublicKey<'a> {
        modulus: UintRef<'a>,
        exponent: UintRef<'a>,
    }
    let spki = &signer.parsed.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != oid::RSA {
        return Err(error("The package's signing key isn't an RSA key"));
    }
    let key =
        RsaPublicKey::from_der(spki.subject_public_key.raw_bytes()).map_err(|e| e.to_string())?;
    let n = num_bigint::BigUint::from_bytes_be(key.modulus.as_bytes());
    let e = num_bigint::BigUint::from_bytes_be(key.exponent.as_bytes());
    let k = key.modulus.as_bytes().len();
    if signature.len() != k || k < 256 {
        return Err(error("RSA signature has the wrong length"));
    }
    let s = num_bigint::BigUint::from_bytes_be(signature);
    if s >= n {
        return Err(error("RSA signature is out of range"));
    }
    let m = s.modpow(&e, &n).to_bytes_be();
    let mut block = vec![0u8; k - m.len()];
    block.extend_from_slice(&m);
    // EMSA-PKCS1-v1_5: 00 01 FF..FF 00 || payload, with at least 8 bytes of FF.
    let payload_start = k - signed.len();
    if payload_start < 11
        || block[0] != 0
        || block[1] != 1
        || block[2..payload_start - 1].iter().any(|b| *b != 0xff)
        || block[payload_start - 1] != 0
        || block[payload_start..] != *signed
    {
        return Err(error("Signature doesn't verify"));
    }
    Ok(())
}

/// The digest of `bytes` with algorithm `id`.
pub(crate) fn digest(id: &ObjectIdentifier, bytes: &[u8]) -> Result<Vec<u8>, String> {
    Ok(match *id {
        oid::SHA1 => sha1::Sha1::digest(bytes).to_vec(),
        oid::SHA256 => sha2::Sha256::digest(bytes).to_vec(),
        oid::SHA384 => sha2::Sha384::digest(bytes).to_vec(),
        oid::SHA512 => sha2::Sha512::digest(bytes).to_vec(),
        other => return Err(format!("Unsupported digest algorithm {other}")),
    })
}

/// The DER `DigestInfo` that PKCS #1 v1.5 signs for `digest`.
pub(crate) fn digest_info(id: &ObjectIdentifier, digest: &[u8]) -> Result<Vec<u8>, String> {
    #[derive(Sequence)]
    struct Info<'a> {
        algorithm: x509_cert::spki::AlgorithmIdentifier<der::asn1::Null>,
        digest: der::asn1::OctetStringRef<'a>,
    }
    Info {
        algorithm: x509_cert::spki::AlgorithmIdentifier {
            oid: *id,
            parameters: Some(der::asn1::Null),
        },
        digest: der::asn1::OctetStringRef::new(digest).map_err(|e| e.to_string())?,
    }
    .to_der()
    .map_err(|e| e.to_string())
}

/// What a chain is trusted for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Installer packages: the leaf must allow package signing or code
    /// signing.
    PackageSigning,
    /// Code: the leaf must allow code signing.
    CodeSigning,
    /// Time-stamp authorities.
    TimeStamping,
}

/// A validated chain, leaf first, ending with an Apple root.
#[derive(Clone, Debug)]
pub struct Chain {
    pub certificates: Vec<Cert>,
}

impl Chain {
    pub fn leaf(&self) -> &Cert {
        &self.certificates[0]
    }

    /// Common names from leaf to root, as `pkgutil --check-signature` lists
    /// them.
    pub fn names(&self) -> Vec<String> {
        self.certificates.iter().map(Cert::common_name).collect()
    }
}

pub fn apple_roots() -> Vec<Cert> {
    APPLE_ROOTS
        .iter()
        .map(|der| Cert::from_der(der).expect("embedded Apple root parses"))
        .collect()
}

/// Builds and validates the chain from `leaf` through `intermediates` to an
/// Apple root, with every certificate valid at `time`.
///
/// Fails closed: an unknown critical extension, a non-CA issuer, a leaf
/// without the extended key usage `purpose` needs, or a chain that doesn't
/// end at an embedded Apple root all fail.
pub fn validate(
    leaf: &Cert,
    intermediates: &[Cert],
    time: SystemTime,
    purpose: Purpose,
) -> Result<Chain, String> {
    let roots = apple_roots();
    let usages = leaf.extended_key_usages();
    let allowed = match purpose {
        Purpose::PackageSigning => {
            usages.contains(&oid::APPLE_PACKAGE_SIGNING) || usages.contains(&oid::CODE_SIGNING)
        }
        Purpose::CodeSigning => usages.contains(&oid::CODE_SIGNING),
        Purpose::TimeStamping => usages.contains(&oid::TIME_STAMPING),
    };
    if !allowed {
        return Err(format!(
            "Certificate {:?} isn't allowed for {}",
            leaf.common_name(),
            match purpose {
                Purpose::PackageSigning => "package signing",
                Purpose::CodeSigning => "code signing",
                Purpose::TimeStamping => "time stamping",
            }
        ));
    }
    let mut chain = vec![leaf.clone()];
    loop {
        let current = chain.last().unwrap().clone();
        current.check_critical_extensions()?;
        if current.not_before() > time || current.not_after() < time {
            return Err(format!(
                "Certificate {:?} isn't valid at the signing time",
                current.common_name()
            ));
        }
        if roots.iter().any(|r| r.der == current.der) {
            return Ok(Chain {
                certificates: chain,
            });
        }
        if chain.len() >= MAX_CHAIN {
            return Err(error("Certificate chain is too long"));
        }
        let issuer = roots
            .iter()
            .chain(intermediates.iter())
            .find(|candidate| candidate.der != current.der && current.signed_by(candidate).is_ok())
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Certificate {:?} doesn't chain to an Apple root certificate",
                    current.common_name()
                )
            })?;
        if !issuer.is_ca() {
            return Err(format!(
                "Certificate {:?} isn't a certificate authority",
                issuer.common_name()
            ));
        }
        chain.push(issuer);
    }
}

/// Seconds since the Unix epoch, for messages and tests.
pub fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
