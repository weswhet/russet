//! Verifies the code signature of a Mach-O binary, the native counterpart
//! of what `codesign --verify` checks for one executable.

use crate::macho::{self, CodeDirectory};
use crate::trust::{self, Chain, Purpose};
use der::asn1::{ObjectIdentifier, OctetStringRef};
use der::Decode;
use std::time::SystemTime;

/// The signed attribute listing every code directory's hash, as a plist.
const CDHASHES: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113635.100.9.1");
/// Code directory flag for ad-hoc signatures.
const CS_ADHOC: u32 = 0x2;

/// What a verified binary's signature says.
#[derive(Clone, Debug)]
pub struct CodeSignature {
    pub identifier: String,
    pub team: Option<String>,
    /// The signing chain, or `None` for an ad-hoc signature.
    pub chain: Option<Chain>,
    /// When the trusted timestamp says the code was signed.
    pub timestamp: Option<SystemTime>,
    /// The hashes of every code directory in every architecture.
    pub cdhashes: Vec<Vec<u8>>,
    /// The preferred code directory hash of the first architecture, which a
    /// parent bundle's resource seal records.
    pub cdhash: Vec<u8>,
}

/// Files outside the binary that its code directories seal.
#[derive(Default)]
pub struct Sealed<'a> {
    /// The bundle's Info.plist (special slot 1).
    pub info_plist: Option<&'a [u8]>,
    /// The bundle's `_CodeSignature/CodeResources` (special slot 3).
    pub resources: Option<&'a [u8]>,
}

fn check_special(
    directory: &CodeDirectory,
    slot: u32,
    actual: Option<&[u8]>,
    what: &str,
) -> Result<(), String> {
    match (directory.special(slot), actual) {
        (None, _) => Ok(()),
        (Some(_), None) => Err(format!("{what} is sealed by the signature but missing")),
        (Some(expected), Some(bytes)) => {
            if macho::hash(directory.hash_type, bytes)? == expected {
                Ok(())
            } else {
                Err(format!("{what} was modified after signing"))
            }
        }
    }
}

/// Reads the `cdhashes` array from the CDHashes plist attribute.
fn signed_cdhashes(values: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, String> {
    let value = values.first().ok_or("Empty CDHashes attribute")?;
    let octets = OctetStringRef::from_der(value).map_err(|e| e.to_string())?;
    let plist = plist::Value::from_reader_xml(octets.as_bytes()).map_err(|e| e.to_string())?;
    plist
        .as_dictionary()
        .and_then(|d| d.get("cdhashes"))
        .and_then(plist::Value::as_array)
        .ok_or("CDHashes attribute has no cdhashes array")?
        .iter()
        .map(|v| {
            v.as_data()
                .map(<[u8]>::to_vec)
                .ok_or_else(|| "CDHashes entry isn't data".to_string())
        })
        .collect()
}

/// Verifies every architecture of a Mach-O binary at time `now`.
pub fn verify_binary(
    file: &[u8],
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let mut result: Option<CodeSignature> = None;
    for slice in macho::slices(file)? {
        let signature = verify_slice(&slice, sealed, now)?;
        match &mut result {
            None => result = Some(signature),
            Some(first) => {
                if first.identifier != signature.identifier {
                    return Err("Architectures are signed with different identifiers".into());
                }
                first.cdhashes.extend(signature.cdhashes);
            }
        }
    }
    result.ok_or_else(|| "code object is not signed at all".into())
}

fn verify_slice(
    slice: &macho::Slice,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let blobs = macho::blobs(slice.signature)?;
    let blob = |slot: u32| blobs.iter().find(|(s, _)| *s == slot).map(|(_, b)| *b);
    let primary = CodeDirectory::parse(
        blob(macho::SLOT_CODE_DIRECTORY).ok_or("Code signature has no code directory")?,
    )?;
    let mut directories = vec![primary.clone()];
    for (slot, bytes) in &blobs {
        if (macho::SLOT_ALTERNATE_DIRECTORIES..macho::SLOT_ALTERNATE_DIRECTORIES + 5).contains(slot)
        {
            directories.push(CodeDirectory::parse(bytes)?);
        }
    }
    for directory in &directories {
        if directory.identifier != primary.identifier {
            return Err("Code directories disagree about the identifier".into());
        }
        if directory.code_limit as usize > slice.signature_offset {
            return Err("Code directory covers the signature itself".into());
        }
        directory.verify_pages(slice.image)?;
        check_special(
            directory,
            1,
            sealed.info_plist.or(slice.info_plist),
            "Info.plist",
        )?;
        check_special(
            directory,
            2,
            blob(macho::SLOT_REQUIREMENTS),
            "The requirements blob",
        )?;
        check_special(
            directory,
            3,
            sealed.resources,
            "The resource seal (CodeResources)",
        )?;
        check_special(
            directory,
            5,
            blob(macho::SLOT_ENTITLEMENTS),
            "The entitlements blob",
        )?;
        check_special(
            directory,
            7,
            blob(macho::SLOT_DER_ENTITLEMENTS),
            "The DER entitlements blob",
        )?;
    }
    let cdhashes = directories
        .iter()
        .map(CodeDirectory::cdhash)
        .collect::<Result<Vec<_>, _>>()?;
    // Prefer the strongest hash, as codesign does.
    let preferred = directories
        .iter()
        .zip(&cdhashes)
        .max_by_key(|(d, _)| match d.hash_type {
            macho::HASH_SHA1 => 0,
            macho::HASH_SHA256_TRUNCATED => 1,
            macho::HASH_SHA256 => 2,
            _ => 3,
        })
        .map(|(_, h)| h.clone())
        .unwrap();

    let cms = blob(macho::SLOT_SIGNATURE)
        .map(|b| b.get(8..).unwrap_or_default())
        .filter(|b| !b.is_empty());
    let adhoc = primary.flags & CS_ADHOC != 0;
    let (chain, timestamp) = match (cms, adhoc) {
        (None, true) => (None, None),
        (None, false) => return Err("Code signature has no CMS signature and isn't ad-hoc".into()),
        (Some(_), true) => return Err("Ad-hoc code signature carries a CMS signature".into()),
        (Some(bytes), false) => {
            let verified = crate::cms::verify_detached(bytes, primary.bytes)?;
            let listed = verified
                .signed_attributes
                .iter()
                .find(|(oid, _)| *oid == CDHASHES)
                .map(|(_, values)| signed_cdhashes(values))
                .transpose()?;
            match listed {
                Some(listed) => {
                    if cdhashes.iter().any(|h| !listed.contains(h)) {
                        return Err("A code directory isn't covered by the signature".into());
                    }
                }
                None if directories.len() > 1 => {
                    return Err("Alternate code directories aren't covered by the signature".into());
                }
                None => {}
            }
            let others: Vec<_> = verified
                .certificates
                .iter()
                .filter(|c| c.der != verified.signer.der)
                .cloned()
                .collect();
            let time = verified.timestamp.unwrap_or(now);
            let chain = trust::validate(&verified.signer, &others, time, Purpose::CodeSigning)?;
            (Some(chain), verified.timestamp)
        }
    };
    Ok(CodeSignature {
        identifier: primary.identifier.clone(),
        team: primary.team.clone(),
        chain,
        timestamp,
        cdhashes,
        cdhash: preferred,
    })
}
