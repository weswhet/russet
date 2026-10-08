//! Verifies the code signature of a Mach-O binary, the native counterpart
//! of what `codesign --verify` checks for one executable.

use crate::macho::{self, CodeDirectory};
use crate::requirement::{Context, Requirement};
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
    /// Each architecture's own signer. The fields above describe the first.
    pub architectures: Vec<Signer>,
}

/// Who signed one architecture of a binary.
#[derive(Clone, Debug)]
pub struct Signer {
    pub identifier: String,
    pub chain: Option<Chain>,
    /// This architecture's code directory hashes.
    pub cdhashes: Vec<Vec<u8>>,
}

impl CodeSignature {
    /// Whether every architecture satisfies `requirement`. `codesign` checks
    /// each architecture against it with its own signer, so one validly
    /// signed architecture can't vouch for another.
    pub fn satisfies(&self, requirement: &Requirement) -> bool {
        !self.architectures.is_empty()
            && self.architectures.iter().all(|signer| {
                requirement.evaluate(&Context {
                    identifier: &signer.identifier,
                    chain: signer.chain.as_ref(),
                    cdhashes: &signer.cdhashes,
                })
            })
    }
}

/// Files outside the binary that its code directories seal.
#[derive(Default)]
pub struct Sealed<'a> {
    /// The bundle's Info.plist (special slot 1).
    pub info_plist: Option<&'a [u8]>,
    /// The bundle's `_CodeSignature/CodeResources` (special slot 3).
    pub resources: Option<&'a [u8]>,
    /// The code is a bundle's own, so its code directories must seal the
    /// bundle's resources and Info.plist. Without that, `CodeResources` and
    /// Info.plist are whatever the bundle ships, and a standalone signed
    /// tool dropped into any bundle would vouch for it.
    pub bundle: bool,
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
                first.architectures.extend(signature.architectures);
            }
        }
    }
    result.ok_or_else(|| "code object is not signed at all".into())
}

/// The parts of one signature, from a Mach-O slice or from the separate
/// files in a bundle's `_CodeSignature` folder.
struct Components<'a> {
    /// Signature blobs by slot, other than the CMS signature.
    blobs: Vec<(u32, &'a [u8])>,
    /// The CMS signature, without any blob header.
    cms: Option<&'a [u8]>,
    /// What the code pages cover, and how far they may reach.
    image: &'a [u8],
    limit: usize,
    /// The Info.plist a standalone binary embeds.
    embedded_info: Option<&'a [u8]>,
    /// The code pages cover the bundle's Info.plist, so no special slot
    /// needs to.
    info_in_pages: bool,
}

fn verify_slice(
    slice: &macho::Slice,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let blobs = macho::blobs(slice.signature)?;
    let cms = blobs
        .iter()
        .find(|(s, _)| *s == macho::SLOT_SIGNATURE)
        .map(|(_, b)| b.get(8..).unwrap_or_default())
        .filter(|b| !b.is_empty());
    verify_components(
        &Components {
            blobs: blobs.clone(),
            cms,
            image: slice.image,
            limit: slice.signature_offset,
            embedded_info: slice.info_plist,
            info_in_pages: false,
        },
        sealed,
        now,
    )
}

/// Verifies a main executable whose signature `codesign` stored as separate
/// files in `_CodeSignature`, because it isn't a Mach-O file. The code pages
/// cover the executable, and the Info.plist is sealed by its special slot.
pub fn verify_detached_code(
    image: &[u8],
    signature_dir: &std::path::Path,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    // Such a file has no signature inside it, so its code directories must
    // cover all of it. Otherwise bytes appended after the covered part would
    // go unchecked.
    for entry in std::fs::read_dir(signature_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("CodeDirectory")
        {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let directory = macho::CodeDirectory::parse(&bytes)?;
        if directory.code_limit != image.len() as u64 {
            return Err("signature doesn't cover the whole executable".into());
        }
    }
    verify_named(
        image,
        |name| std::fs::read(signature_dir.join(name)).ok(),
        sealed,
        false,
        now,
    )
}

/// Verifies a bundle without a main executable, whose signature is stored
/// as separate files in `_CodeSignature` and whose code pages cover its
/// Info.plist, as `codesign` signs such bundles.
pub fn verify_detached(
    signature_dir: &std::path::Path,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let info = sealed
        .info_plist
        .ok_or("A bundle without an executable has no Info.plist")?;
    verify_named(
        info,
        |name| std::fs::read(signature_dir.join(name)).ok(),
        sealed,
        true,
        now,
    )
}

/// Verifies a non-Mach-O file signed as code, from its signature components
/// by name (`CodeDirectory`, `CodeSignature`, and so on), which `codesign`
/// stores in the file's `com.apple.cs.*` extended attributes.
pub fn verify_components_from(
    image: &[u8],
    read: impl Fn(&str) -> Option<Vec<u8>>,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    if read("CodeDirectory").is_none() {
        return Err("code object is not signed at all".into());
    }
    verify_named(image, read, &Sealed::default(), false, now)
}

/// Verifies a signature whose components are stored by name rather than in
/// a Mach-O superblob; the code pages cover `image`.
fn verify_named(
    image: &[u8],
    read: impl Fn(&str) -> Option<Vec<u8>>,
    sealed: &Sealed,
    info_in_pages: bool,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let mut owned: Vec<(u32, Vec<u8>)> = Vec::new();
    for (name, slot) in [
        ("CodeDirectory", macho::SLOT_CODE_DIRECTORY),
        ("CodeRequirements", macho::SLOT_REQUIREMENTS),
        ("CodeEntitlements", macho::SLOT_ENTITLEMENTS),
        ("CodeEntitlementsDER", macho::SLOT_DER_ENTITLEMENTS),
    ] {
        if let Some(bytes) = read(name) {
            owned.push((slot, bytes));
        }
    }
    // codesign stores alternate code directories as CodeRequirements-1,
    // CodeRequirements-2, and so on.
    for index in 0..5 {
        if let Some(bytes) = read(&format!("CodeRequirements-{}", index + 1)) {
            owned.push((macho::SLOT_ALTERNATE_DIRECTORIES + index, bytes));
        }
    }
    let cms = read("CodeSignature").filter(|b| !b.is_empty());
    verify_components(
        &Components {
            blobs: owned.iter().map(|(s, b)| (*s, b.as_slice())).collect(),
            cms: cms.as_deref(),
            image,
            limit: image.len(),
            embedded_info: None,
            info_in_pages,
        },
        sealed,
        now,
    )
}

fn verify_components(
    components: &Components,
    sealed: &Sealed,
    now: SystemTime,
) -> Result<CodeSignature, String> {
    let blobs = &components.blobs;
    let blob = |slot: u32| blobs.iter().find(|(s, _)| *s == slot).map(|(_, b)| *b);
    let primary = CodeDirectory::parse(
        blob(macho::SLOT_CODE_DIRECTORY).ok_or("Code signature has no code directory")?,
    )?;
    let mut directories = vec![primary.clone()];
    for (slot, bytes) in blobs {
        if (macho::SLOT_ALTERNATE_DIRECTORIES..macho::SLOT_ALTERNATE_DIRECTORIES + 5).contains(slot)
        {
            directories.push(CodeDirectory::parse(bytes)?);
        }
    }
    for directory in &directories {
        if directory.identifier != primary.identifier {
            return Err("Code directories disagree about the identifier".into());
        }
        if directory.code_limit as usize > components.limit {
            return Err("Code directory covers the signature itself".into());
        }
        directory.verify_pages(components.image)?;
        if sealed.bundle {
            if directory.special(3).is_none() {
                return Err(
                    "code has no resources but signature indicates they must be present".into(),
                );
            }
            if directory.special(1).is_none() && !components.info_in_pages {
                return Err("Info.plist isn't bound to the signature".into());
            }
        }
        check_special(
            directory,
            1,
            sealed.info_plist.or(components.embedded_info),
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

    let cms = components.cms;
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
            // Apple checks the chain at the trusted timestamp, or else at the
            // signed signing time, which is how a Developer ID signature from
            // before its certificate expired stays valid.
            let time = verified.timestamp.or(verified.signing_time).unwrap_or(now);
            let chain = trust::validate(&verified.signer, &others, time, Purpose::CodeSigning)?;
            (Some(chain), verified.timestamp)
        }
    };
    let architectures = vec![Signer {
        identifier: primary.identifier.clone(),
        chain: chain.clone(),
        cdhashes: cdhashes.clone(),
    }];
    Ok(CodeSignature {
        identifier: primary.identifier.clone(),
        team: primary.team.clone(),
        chain,
        timestamp,
        cdhashes,
        cdhash: preferred,
        architectures,
    })
}
