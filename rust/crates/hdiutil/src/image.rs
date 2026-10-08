use russet_fs::{Limits, SkippedXattr, TreeWriter};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use udif::format::BlockType;

/// What `hdiutil imageinfo` reports about an image, as far as Russet needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// The image format, such as `UDZO` or `UDRO`.
    pub format: String,
    /// Whether the image carries a software license agreement, which
    /// `hdiutil attach` would ask to accept. Extraction accepts it, as
    /// Russet's Apple backend does.
    pub has_license: bool,
}

/// The result of extracting an image: one folder per mountable volume, in
/// the order `hdiutil attach` reports them.
#[derive(Debug, Default)]
pub struct Extraction {
    pub volumes: Vec<PathBuf>,
    /// Extended attributes the host filesystem refused.
    pub skipped_xattrs: Vec<SkippedXattr>,
}

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn udif_error(path: &Path, error: udif::DppError) -> io::Error {
    match error {
        udif::DppError::Io(e) => e,
        other => invalid(format!("Can't read disk image {}: {other}", path.display())),
    }
}

/// Rejects images Russet can't read with a message that says why.
fn check_supported(path: &Path) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic)?;
    if read == 8 && (&magic == b"encrcdsa" || &magic == b"cdsaencr") {
        return Err(invalid(format!(
            "{} is an encrypted disk image, which Russet can't open natively",
            path.display()
        )));
    }
    let length = file.seek(SeekFrom::End(0))?;
    if length >= 512 {
        let mut trailer = [0u8; 4];
        file.seek(SeekFrom::End(-512))?;
        file.read_exact(&mut trailer)?;
        if &trailer == b"koly" {
            return Ok(());
        }
    }
    if length >= 0x8006 {
        let mut iso = [0u8; 5];
        file.seek(SeekFrom::Start(0x8001))?;
        file.read_exact(&mut iso)?;
        if &iso == b"CD001" {
            return Err(invalid(format!(
                "{} is an ISO 9660 image, which Russet can't open natively yet",
                path.display()
            )));
        }
    }
    Err(invalid(format!(
        "{} isn't a disk image that Russet can open natively",
        path.display()
    )))
}

/// Reports an image's format the way `hdiutil imageinfo` names it.
pub fn image_info(path: &Path) -> io::Result<ImageInfo> {
    check_supported(path)?;
    let reader =
        udif::DmgReader::new(BufReader::new(File::open(path)?)).map_err(|e| udif_error(path, e))?;
    let mut kinds = Vec::new();
    for partition in reader.partitions() {
        for run in &partition.block_map.block_runs {
            if !kinds.contains(&run.block_type) {
                kinds.push(run.block_type);
            }
        }
    }
    let format = if kinds.contains(&BlockType::Zlib) {
        "UDZO"
    } else if kinds.contains(&BlockType::Bzip2) {
        "UDBZ"
    } else if kinds.contains(&BlockType::Lzfse) {
        "ULFO"
    } else if kinds.contains(&BlockType::Xz) {
        "ULMO"
    } else if kinds.contains(&BlockType::Adc) {
        "UDCO"
    } else {
        "UDRO"
    };
    Ok(ImageInfo {
        format: format.into(),
        has_license: plist_has_license(path, reader.koly())?,
    })
}

/// A software license agreement is an `LPic` resource in the image's
/// property list.
fn plist_has_license(path: &Path, koly: &udif::KolyHeader) -> io::Result<bool> {
    if koly.plist_length == 0 {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(koly.plist_offset))?;
    let mut xml = Vec::new();
    file.take(koly.plist_length.min(64 << 20))
        .read_to_end(&mut xml)?;
    let key = b"<key>LPic</key>";
    Ok(xml.windows(key.len()).any(|w| w == key))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filesystem {
    Hfs,
    Apfs,
}

fn sniff(file: &mut File) -> io::Result<Option<Filesystem>> {
    let mut header = [0u8; 1026];
    file.seek(SeekFrom::Start(0))?;
    let read = file.read(&mut header)?;
    if read >= 36 && &header[32..36] == b"NXSB" {
        return Ok(Some(Filesystem::Apfs));
    }
    if read >= 1026 && (&header[1024..1026] == b"H+" || &header[1024..1026] == b"HX") {
        return Ok(Some(Filesystem::Hfs));
    }
    Ok(None)
}

/// Extracts every HFS+ and APFS volume in `image` into numbered folders
/// under `destination`, which must exist and be empty.
///
/// Each partition is decompressed into an unlinked temporary file inside
/// `destination`, so extraction needs free space for the largest partition
/// plus the extracted files.
pub fn extract(image: &Path, destination: &Path, limits: Limits) -> io::Result<Extraction> {
    check_supported(image)?;
    let mut reader = udif::DmgReader::new(BufReader::new(File::open(image)?))
        .map_err(|e| udif_error(image, e))?;
    // A partition is decompressed whole before its files are read, so its
    // declared size counts against the extraction's total.
    if reader
        .partitions()
        .iter()
        .any(|p| p.block_map.sector_count.saturating_mul(512) > limits.max_total_bytes)
    {
        return Err(invalid(format!(
            "{} declares a volume larger than the extraction limit",
            image.display()
        )));
    }
    let ids: Vec<i32> = reader.partitions().iter().map(|p| p.id).collect();
    let mut extraction = Extraction::default();
    for id in ids {
        let mut partition = tempfile::tempfile_in(destination)?;
        reader
            .decompress_partition_to(id, &mut partition)
            .map_err(|e| udif_error(image, e))?;
        let Some(filesystem) = sniff(&mut partition)? else {
            continue;
        };
        let root = destination.join(format!("volume-{}", extraction.volumes.len() + 1));
        std::fs::create_dir(&root)?;
        let mut writer = TreeWriter::open(&root, limits)?;
        partition.seek(SeekFrom::Start(0))?;
        let partition = BufReader::new(partition);
        match filesystem {
            Filesystem::Hfs => crate::hfs_volume::extract(partition, &mut writer)?,
            Filesystem::Apfs => crate::apfs_volume::extract(partition, &mut writer)?,
        }
        extraction.skipped_xattrs.extend(writer.finish()?);
        extraction.volumes.push(root);
    }
    if extraction.volumes.is_empty() {
        return Err(invalid(format!(
            "{} has no HFS+ or APFS volume",
            image.display()
        )));
    }
    Ok(extraction)
}
