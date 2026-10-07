//! Russet's replacements for the tools icon extraction uses on macOS:
//! `lsbom -s`, `tar -xOf`, `pkgutil --expand`, and ImageIO's ICNS-to-PNG
//! conversion.

use icns::{IconFamily, IconType, PixelFormat};
use russet_fs::Limits;
use std::io::BufReader;
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

/// One regular file from a package payload. Apple Archive payloads aren't
/// supported, so they read as missing, as a failed `aa extract` does.
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

/// Converts the icon Munki would choose from an `.icns` file to PNG.
pub(crate) fn convert_to_png(source: &Path, destination: &Path) -> Result<(), String> {
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let family =
        IconFamily::read(BufReader::new(file)).map_err(|_| "Cannot read icon image".to_owned())?;
    let chosen =
        select(&representations(&family)).ok_or("Icon image contains no representations")?;
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
        let mut decoder = png::Decoder::new(BufReader::new(std::fs::File::open(path).unwrap()));
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
