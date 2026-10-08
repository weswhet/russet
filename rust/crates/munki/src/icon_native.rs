//! Russet's replacements for the tools icon extraction uses on macOS:
//! `lsbom -s`, `tar -xOf`, `pkgutil --expand`, and ImageIO's ICNS-to-PNG
//! conversion.

use icns::{IconFamily, IconType, PixelFormat};
use russet_fs::Limits;
use std::io::Read;
use std::path::Path;

/// Icon resources and the Info.plist files that name them are small; a
/// payload member larger than this isn't read.
const MAX_MEMBER_BYTES: u64 = 64 << 20;

/// The paths in a BOM, like `lsbom -s`.
pub(crate) fn bom_paths(bom: &Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(bom).map_err(|e| e.to_string())?;
    Ok(russet_mkbom::read(&bytes)?
        .into_iter()
        .map(|entry| {
            if entry.path.is_empty() {
                ".".to_owned()
            } else {
                format!("./{}", entry.path)
            }
        })
        .collect())
}

/// One regular file from a package payload: cpio (plain, gzip, or pbzx) or
/// an Apple Archive.
pub(crate) fn payload_member(archive: &Path, entry: &Path) -> Result<Vec<u8>, String> {
    russet_ditto::read_cpio_member(archive, entry, MAX_MEMBER_BYTES)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{} isn't in the payload", entry.display()))
}

/// `pkgutil --expand`.
pub(crate) fn expand(package: &Path, destination: &Path) -> Result<(), String> {
    russet_pkgutil::expand(package, destination, Limits::default()).map_err(|e| e.to_string())
}

/// One image in an icon file, described the way ImageIO reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Representation {
    height: u32,
    dpi: u32,
    kind: IconType,
}

/// The images ImageIO lists for an icon family: one per size and density,
/// largest first and 144 dpi before 72 dpi at the same size. Masks are
/// merged into their images.
fn representations(family: &IconFamily) -> Vec<Representation> {
    let mut out: Vec<Representation> = family
        .available_icons()
        .into_iter()
        .map(|kind| Representation {
            height: kind.pixel_height(),
            dpi: 72 * kind.pixel_density(),
            kind,
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse((r.height, r.dpi)));
    out.dedup_by_key(|r| (r.height, r.dpi));
    out
}

/// Munki's choice: 512 pixels at 72 dpi, then any 512-pixel image, then
/// the smallest larger image (72 dpi first), then the largest.
fn select(representations: &[Representation]) -> Option<Representation> {
    let find = |f: &dyn Fn(&Representation) -> bool| representations.iter().copied().find(f);
    if let Some(r) = find(&|r| r.height == 512 && r.dpi == 72) {
        return Some(r);
    }
    if let Some(r) = find(&|r| r.height == 512) {
        return Some(r);
    }
    let mut ascending = representations.to_vec();
    ascending.sort_by_key(|r| (r.height, r.dpi));
    ascending
        .iter()
        .copied()
        .find(|r| r.height > 512 && r.dpi == 72)
        .or_else(|| ascending.iter().copied().find(|r| r.height > 512))
        .or_else(|| ascending.last().copied())
}

/// Real icon files are a few MiB; a larger one isn't read.
const MAX_ICON_BYTES: u64 = 64 << 20;

fn be32(bytes: &[u8], at: usize) -> Option<usize> {
    let field = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(field.try_into().ok()?) as usize)
}

/// Checks that every element the `icns` reader will read fits in the file.
/// The reader allocates each element's declared length before reading it,
/// so a short file could otherwise ask for 4 GiB.
fn check_elements(bytes: &[u8]) -> Result<(), String> {
    const INVALID: &str = "Cannot read icon image";
    if bytes.get(..4) != Some(b"icns") {
        return Err(INVALID.into());
    }
    let declared = be32(bytes, 4).ok_or(INVALID)?;
    let mut position = 8;
    while position < declared {
        let length = be32(bytes, position + 4).ok_or(INVALID)?;
        if length < 8 || length > bytes.len() - position {
            return Err(INVALID.into());
        }
        position += length;
    }
    Ok(())
}

/// Checks that a PNG-encoded element is the size its icon type says before
/// it's decoded; the `icns` reader decodes first and compares afterward, so
/// a PNG claiming huge dimensions would allocate their pixels.
fn check_png_size(family: &IconFamily, kind: IconType) -> Result<(), String> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let Some(element) = family.elements.iter().find(|e| e.ostype == kind.ostype()) else {
        return Ok(());
    };
    let data = &element.data;
    if !data.starts_with(PNG) {
        return Ok(());
    }
    let size = (be32(data, 16), be32(data, 20));
    if data.get(12..16) != Some(b"IHDR")
        || size
            != (
                Some(kind.pixel_width() as usize),
                Some(kind.pixel_height() as usize),
            )
    {
        return Err("Cannot decode icon representation".into());
    }
    Ok(())
}

/// Converts the icon Munki would choose from an `.icns` file to PNG.
pub(crate) fn convert_to_png(source: &Path, destination: &Path) -> Result<(), String> {
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_ICON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ICON_BYTES {
        return Err("Icon image is too large".into());
    }
    check_elements(&bytes)?;
    let family =
        IconFamily::read(bytes.as_slice()).map_err(|_| "Cannot read icon image".to_owned())?;
    let chosen =
        select(&representations(&family)).ok_or("Icon image contains no representations")?;
    check_png_size(&family, chosen.kind)?;
    let image = family
        .get_icon_with_type(chosen.kind)
        .map_err(|_| "Cannot decode icon representation".to_owned())?
        .convert_to(PixelFormat::RGBA);
    let output = std::fs::File::create(destination).map_err(|e| e.to_string())?;
    image
        .write_png(std::io::BufWriter::new(output))
        .map_err(|_| "Cannot write PNG icon".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(height: u32, dpi: u32) -> Representation {
        Representation {
            height,
            dpi,
            kind: IconType::RGBA32_512x512,
        }
    }

    #[test]
    fn selects_like_munki() {
        let modern = [
            rep(1024, 144),
            rep(512, 144),
            rep(512, 72),
            rep(256, 144),
            rep(128, 72),
        ];
        assert_eq!(select(&modern), Some(rep(512, 72)));
        assert_eq!(select(&modern[..2]), Some(rep(512, 144)));
        assert_eq!(
            select(&[rep(1024, 144), rep(256, 72)]),
            Some(rep(1024, 144))
        );
        assert_eq!(select(&[rep(256, 144), rep(128, 72)]), Some(rep(256, 144)));
        assert_eq!(select(&[]), None);
    }

    /// An app icon goes through a package built natively and comes out as
    /// the 512-pixel PNG.
    #[test]
    fn extracts_icon_from_package() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let resources = root.join("Thing.app/Contents/Resources");
        std::fs::create_dir_all(&resources).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "com.example.thing".into());
        info.insert("CFBundleIconFile".into(), "AppIcon".into());
        plist::Value::Dictionary(info)
            .to_file_xml(root.join("Thing.app/Contents/Info.plist"))
            .unwrap();
        let mut family = IconFamily::new();
        for size in [256, 512] {
            let mut image = icns::Image::new(PixelFormat::RGBA, size, size);
            image.data_mut().fill((size / 4) as u8);
            family.add_icon(&image).unwrap();
        }
        family
            .write(std::fs::File::create(resources.join("AppIcon.icns")).unwrap())
            .unwrap();
        let package = temp.path().join("Thing.pkg");
        let nodes = russet_pkgbuild::collect(&root, &[]).unwrap();
        russet_pkgbuild::build(
            &nodes,
            &russet_pkgbuild::Options {
                identifier: "com.example.thing",
                version: "1.0",
                install_location: Some("/Applications"),
                min_os_version: None,
                scripts: None,
                info_template: None,
                components: &[],
            },
            &package,
        )
        .unwrap();

        let expanded = temp.path().join("expanded");
        expand(&package, &expanded).unwrap();
        let bom = expanded.join("Bom");
        let paths = bom_paths(&bom).unwrap();
        assert_eq!(paths[0], ".");
        assert!(paths.contains(&"./Thing.app/Contents/Info.plist".to_owned()));
        #[cfg(target_os = "macos")]
        {
            let listing = std::process::Command::new("/usr/bin/lsbom")
                .arg("-s")
                .arg(&bom)
                .output()
                .unwrap();
            let expected: Vec<String> = String::from_utf8(listing.stdout)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect();
            assert_eq!(paths, expected);
        }
        let icon = payload_member(
            &expanded.join("Payload"),
            Path::new("./Thing.app/Contents/Resources/AppIcon.icns"),
        )
        .unwrap();
        assert_eq!(icon, std::fs::read(resources.join("AppIcon.icns")).unwrap());
        assert!(payload_member(&expanded.join("Payload"), Path::new("Missing")).is_err());
        let png = temp.path().join("icon.png");
        convert_to_png(&resources.join("AppIcon.icns"), &png).unwrap();
        let (width, height, pixels) = pixels(&png);
        assert_eq!((width, height), (512, 512));
        assert_eq!(&pixels[..4], &[128; 4]);
    }

    fn pixels(path: &Path) -> (u32, u32, Vec<u8>) {
        let mut decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(info.buffer_size());
        let rgba = match info.color_type {
            png::ColorType::Rgba => buffer,
            png::ColorType::Rgb => buffer
                .chunks(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buffer
                .chunks(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            other => panic!("unexpected PNG color type {other:?}"),
        };
        (info.width, info.height, rgba)
    }

    /// The native conversion picks the same image as ImageIO for the icons
    /// in CoreTypes, with the same pixels.
    #[cfg(target_os = "macos")]
    #[test]
    fn matches_imageio() {
        let dir = Path::new("/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources");
        let temp = tempfile::tempdir().unwrap();
        let mut checked = 0;
        let mut differences = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "icns"))
            .collect();
        entries.sort();
        // A quarter of them keeps the test quick; all 257 matched on macOS 27.
        for icon in entries.into_iter().step_by(4) {
            let apple = temp.path().join("apple.png");
            let native = temp.path().join("native.png");
            crate::icons::imageio_convert_to_png(&icon, &apple).unwrap();
            convert_to_png(&icon, &native).unwrap();
            let (aw, ah, a) = pixels(&apple);
            let (nw, nh, n) = pixels(&native);
            checked += 1;
            if (aw, ah) != (nw, nh) {
                differences.push(format!("{}: {aw}x{ah} vs {nw}x{nh}", icon.display()));
            } else {
                // ImageIO stores premultiplied color, which loses precision
                // in faint pixels, so compare premultiplied values and
                // allow rounding.
                let premultiplied = |p: &[u8]| {
                    let a = u32::from(p[3]);
                    [0, 1, 2].map(|i| ((u32::from(p[i]) * a + 127) / 255) as u8)
                };
                let max = a
                    .chunks(4)
                    .zip(n.chunks(4))
                    .map(|(x, y)| {
                        let color = premultiplied(x)
                            .iter()
                            .zip(premultiplied(y))
                            .map(|(p, q)| p.abs_diff(q).saturating_sub(1))
                            .max()
                            .unwrap();
                        color.max(x[3].abs_diff(y[3]))
                    })
                    .max()
                    .unwrap_or(0);
                if max > 0 {
                    differences.push(format!("{}: pixels differ by up to {max}", icon.display()));
                }
            }
        }
        assert!(checked > 0);
        assert!(
            differences.is_empty(),
            "{checked} checked:\n{}",
            differences.join("\n")
        );
    }
}

#[cfg(test)]
mod limits {
    use super::*;

    fn convert(bytes: &[u8]) -> Result<(), String> {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("in.icns");
        std::fs::write(&source, bytes).unwrap();
        convert_to_png(&source, &temp.path().join("out.png"))
    }

    /// An element claiming 4 GiB in a 16-byte file is refused before the
    /// `icns` reader allocates it.
    #[test]
    fn refuses_element_longer_than_file() {
        let mut bytes = b"icns\xff\xff\xff\xffic10\xff\xff\xff\xff".to_vec();
        assert!(check_elements(&bytes).is_err());
        assert!(convert(&bytes).is_err());
        bytes.truncate(8);
        bytes[4..8].copy_from_slice(&8u32.to_be_bytes());
        assert!(check_elements(&bytes).is_ok());
    }

    /// A 512-pixel element whose PNG claims 65535x65535 isn't decoded.
    #[test]
    fn refuses_png_larger_than_its_icon_type() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&65535u32.to_be_bytes());
        png.extend_from_slice(&65535u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        let mut bytes = b"icns".to_vec();
        bytes.extend_from_slice(&((16 + png.len()) as u32).to_be_bytes());
        bytes.extend_from_slice(b"ic09");
        bytes.extend_from_slice(&((8 + png.len()) as u32).to_be_bytes());
        bytes.extend_from_slice(&png);
        assert_eq!(
            convert(&bytes).unwrap_err(),
            "Cannot decode icon representation"
        );
    }
}
