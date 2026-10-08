//! Detached CMS signatures and the RFC 3161 timestamps inside them.

use crate::trust::{self, digest, verify_message, Cert, Purpose};
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::{ObjectIdentifier, OctetStringRef};
use der::{Decode, Encode};
use std::time::{SystemTime, UNIX_EPOCH};

const SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
const TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");

/// A CMS signature whose signer signed the expected content.
#[derive(Clone, Debug)]
pub struct VerifiedCms {
    pub signer: Cert,
    /// Every certificate the signature carries.
    pub certificates: Vec<Cert>,
    /// The time from a verified timestamp, if the signature has one.
    pub timestamp: Option<SystemTime>,
    /// The signed `signingTime` attribute, if the signature has one. Without
    /// a timestamp, it is the time the signer claims, which is covered by the
    /// signature.
    pub signing_time: Option<SystemTime>,
    /// Signed attributes, as attribute type and DER values.
    pub signed_attributes: Vec<(ObjectIdentifier, Vec<Vec<u8>>)>,
}

/// A decoded `SignedData`, with the first signer's signed attributes as
/// they were encoded.
struct Parsed {
    data: SignedData,
    /// The signed attributes' encoding with the SET tag, which is what the
    /// signer signed. Re-encoding them would sort them into DER order, which
    /// some signers don't use.
    signed_attributes: Option<Vec<u8>>,
}

/// Decodes a CMS `ContentInfo`, ignoring the zero padding that xar and code
/// signatures add after it.
fn signed_data(bytes: &[u8]) -> Result<Parsed, String> {
    let (der, used) = crate::ber::to_der(bytes)?;
    if bytes[used..].iter().any(|b| *b != 0) {
        return Err("CMS signature is followed by unexpected data".into());
    }
    let info = ContentInfo::from_der(&der).map_err(|e| format!("Invalid CMS signature: {e}"))?;
    if info.content_type != SIGNED_DATA {
        return Err("CMS signature isn't SignedData".into());
    }
    let data = info
        .content
        .decode_as::<SignedData>()
        .map_err(|e| format!("Invalid CMS SignedData: {e}"))?;
    Ok(Parsed {
        data,
        signed_attributes: raw_signed_attributes(&der),
    })
}

/// Finds the first SignerInfo's `[0] IMPLICIT` signed attributes in a DER
/// `ContentInfo` and returns them tagged as a SET.
fn raw_signed_attributes(der: &[u8]) -> Option<Vec<u8>> {
    use der::asn1::AnyRef;
    use der::{Reader, SliceReader, Tag, TagNumber, Tagged};
    fn children(value: &[u8]) -> Option<Vec<AnyRef<'_>>> {
        let mut reader = SliceReader::new(value).ok()?;
        let mut out = Vec::new();
        while !reader.is_finished() {
            out.push(reader.decode::<AnyRef>().ok()?);
        }
        Some(out)
    }
    let content_info = AnyRef::from_der(der).ok()?;
    let explicit = children(content_info.value())?.into_iter().nth(1)?;
    let signed_data = children(explicit.value())?.into_iter().next()?;
    let signer_infos = children(signed_data.value())?
        .into_iter()
        .rfind(|c| c.tag() == Tag::Set)?;
    let signer = children(signer_infos.value())?.into_iter().next()?;
    let attributes = children(signer.value())?.into_iter().find(|c| {
        c.tag()
            == Tag::ContextSpecific {
                constructed: true,
                number: TagNumber::N0,
            }
    })?;
    let mut out = attributes.to_der().ok()?;
    out[0] = 0x31;
    Some(out)
}

fn certificates(data: &SignedData) -> Result<Vec<Cert>, String> {
    let mut out = Vec::new();
    for choice in data.certificates.iter().flat_map(|set| set.0.iter()) {
        if let cms::cert::CertificateChoices::Certificate(cert) = choice {
            out.push(Cert::from_der(&cert.to_der().map_err(|e| e.to_string())?)?);
        }
    }
    Ok(out)
}

fn signer_certificate(info: &SignerInfo, certificates: &[Cert]) -> Result<Cert, String> {
    let SignerIdentifier::IssuerAndSerialNumber(id) = &info.sid else {
        return Err("CMS signer is identified by key ID, which isn't supported".into());
    };
    certificates
        .iter()
        .find(|c| {
            c.parsed.tbs_certificate.issuer == id.issuer
                && c.parsed.tbs_certificate.serial_number == id.serial_number
        })
        .cloned()
        .ok_or_else(|| "CMS signer certificate is missing".into())
}

/// Checks one signer: its signed attributes must carry the digest of
/// `content`, and its signature must cover them.
fn check_signer(
    info: &SignerInfo,
    raw_attributes: Option<&[u8]>,
    signer: &Cert,
    content: &[u8],
) -> Result<(), String> {
    let attributes = info
        .signed_attrs
        .as_ref()
        .ok_or("CMS signature has no signed attributes")?;
    let expected = digest(&info.digest_alg.oid, content)?;
    let mut digests = attributes
        .iter()
        .filter(|a| a.oid == MESSAGE_DIGEST)
        .flat_map(|a| a.values.iter());
    let value = digests
        .next()
        .ok_or("CMS signature has no message digest")?;
    if digests.next().is_some() {
        return Err("CMS signature has more than one message digest".into());
    }
    let actual = value
        .decode_as::<OctetStringRef>()
        .map_err(|e| e.to_string())?;
    if actual.as_bytes() != expected.as_slice() {
        return Err("CMS message digest doesn't match the signed content".into());
    }
    let signed = match raw_attributes {
        Some(raw) => raw.to_vec(),
        None => attributes.to_der().map_err(|e| e.to_string())?,
    };
    verify_message(
        signer,
        &info.signature_algorithm.oid,
        Some(&info.digest_alg.oid),
        &signed,
        info.signature.as_bytes(),
    )
}

/// Verifies a detached CMS signature over `content`. A timestamp in the
/// signature is verified, including its Apple-anchored chain, and its time
/// returned. The signer's own chain isn't validated here.
pub fn verify_detached(bytes: &[u8], content: &[u8]) -> Result<VerifiedCms, String> {
    let parsed = signed_data(bytes)?;
    let data = &parsed.data;
    let certificates = certificates(data)?;
    let mut signers = data.signer_infos.0.iter();
    let info = signers.next().ok_or("CMS signature has no signer")?;
    if signers.next().is_some() {
        return Err("CMS signature has more than one signer".into());
    }
    let signer = signer_certificate(info, &certificates)?;
    check_signer(info, parsed.signed_attributes.as_deref(), &signer, content)?;
    let mut timestamp = None;
    for attribute in info.unsigned_attrs.iter().flat_map(|a| a.iter()) {
        if attribute.oid == TIMESTAMP_TOKEN {
            let token = attribute.values.get(0).ok_or("Empty timestamp attribute")?;
            let token = token.to_der().map_err(|e| e.to_string())?;
            timestamp = Some(verify_timestamp(&token, info.signature.as_bytes())?);
        }
    }
    let signed_attributes = info
        .signed_attrs
        .iter()
        .flat_map(|a| a.iter())
        .map(|a| {
            let values = a.values.iter().filter_map(|v| v.to_der().ok()).collect();
            (a.oid, values)
        })
        .collect();
    let signing_time = info
        .signed_attrs
        .iter()
        .flat_map(|a| a.iter())
        .find(|a| a.oid == SIGNING_TIME)
        .and_then(|a| a.values.get(0))
        .and_then(|v| v.decode_as::<der::asn1::UtcTime>().ok())
        .map(|t| UNIX_EPOCH + t.to_unix_duration());
    Ok(VerifiedCms {
        signer,
        certificates,
        timestamp,
        signing_time,
        signed_attributes,
    })
}

/// Verifies an RFC 3161 timestamp token over `signature` and returns its
/// time.
fn verify_timestamp(token: &[u8], signature: &[u8]) -> Result<SystemTime, String> {
    let parsed = signed_data(token)?;
    let data = &parsed.data;
    if data.encap_content_info.econtent_type != TST_INFO {
        return Err("Timestamp token doesn't hold TSTInfo".into());
    }
    let content = data
        .encap_content_info
        .econtent
        .as_ref()
        .ok_or("Timestamp token has no content")?
        .decode_as::<OctetStringRef>()
        .map_err(|e| e.to_string())?;
    // Only the message imprint and time are needed. TSTInfo is parsed by
    // hand because some authorities use policy OIDs, such as 1.2.3, that the
    // strict OID type rejects.
    let (imprint_algorithm, imprint_hash, gen_time) = tst_info(content.as_bytes())?;
    let imprint = digest(&imprint_algorithm, signature)?;
    if imprint_hash != imprint {
        return Err("Timestamp doesn't cover this signature".into());
    }
    let certificates = certificates(data)?;
    let info = data
        .signer_infos
        .0
        .get(0)
        .ok_or("Timestamp token has no signer")?;
    let signer = signer_certificate(info, &certificates)?;
    check_signer(
        info,
        parsed.signed_attributes.as_deref(),
        &signer,
        content.as_bytes(),
    )?;
    let time = UNIX_EPOCH + gen_time.to_unix_duration();
    let others: Vec<Cert> = certificates
        .into_iter()
        .filter(|c| c.der != signer.der)
        .collect();
    trust::validate(&signer, &others, time, Purpose::TimeStamping)?;
    Ok(time)
}

/// Reads the message imprint and generation time from a TSTInfo.
fn tst_info(
    bytes: &[u8],
) -> Result<(ObjectIdentifier, Vec<u8>, der::asn1::GeneralizedTime), String> {
    use der::{Reader, Tagged};
    let invalid = |e: der::Error| format!("Invalid TSTInfo: {e}");
    let sequence = der::asn1::AnyRef::from_der(bytes).map_err(invalid)?;
    if sequence.tag() != der::Tag::Sequence {
        return Err("Invalid TSTInfo: not a sequence".into());
    }
    let mut reader = der::SliceReader::new(sequence.value()).map_err(invalid)?;
    let _version: der::asn1::AnyRef = reader.decode().map_err(invalid)?;
    let _policy: der::asn1::AnyRef = reader.decode().map_err(invalid)?;
    let imprint: x509_tsp::MessageImprint = reader.decode().map_err(invalid)?;
    let _serial: der::asn1::AnyRef = reader.decode().map_err(invalid)?;
    let time: der::asn1::GeneralizedTime = reader.decode().map_err(invalid)?;
    Ok((
        imprint.hash_algorithm.oid,
        imprint.hashed_message.as_bytes().to_vec(),
        time,
    ))
}
