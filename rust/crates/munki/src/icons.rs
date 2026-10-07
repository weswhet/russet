//! Native ImageIO icon selection matching Munki's 512px/72dpi preference.
use autopkg_platform::backend::{select, Backend, Tool};
use plist::{Dictionary, Value};
use std::path::{Path, PathBuf};

#[cfg(unix)]
fn native() -> bool {
    select(Tool::Icons) == Backend::Native
}

fn app_icon(app: &Path) -> Option<PathBuf> {
    let info = Value::from_file(app.join("Contents/Info.plist"))
        .ok()?
        .into_dictionary()?;
    let filename = info
        .get("CFBundleIconFile")
        .and_then(Value::as_string)
        .or_else(|| info.get("CFBundleIconName").and_then(Value::as_string))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            app.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    if Path::new(&filename)
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    let filename = if Path::new(&filename).extension().is_none() {
        format!("{filename}.icns")
    } else {
        filename
    };
    let icon = app.join("Contents/Resources").join(filename);
    if !icon.is_file()
        || !icon
            .canonicalize()
            .ok()?
            .starts_with(app.canonicalize().ok()?)
    {
        return None;
    }
    Some(icon)
}
fn bundle_icons(package: &Path, temporary: &Path) -> Result<Vec<PathBuf>, String> {
    let bom = package.join("Contents/Archive.bom");
    let archive = package.join("Contents/Archive.pax.gz");
    // The pinned reference only extracts direct bundle payloads with a BOM.
    archive_icons(&bom, &archive, temporary)
}

fn archive_file(archive: &Path, entry: &Path) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    if native() {
        return crate::icon_native::payload_member(archive, entry);
    }
    if let Ok(bytes) = crate::metadata::command(
        "/usr/bin/tar",
        &["-xOf".as_ref(), archive.as_os_str(), entry.as_os_str()],
    ) {
        return Ok(bytes);
    }
    // Newer package payloads use Apple Archive. Extract only the requested
    // regular file into an empty directory, never symlinks or package code.
    let normalized: PathBuf = entry
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(c) => Some(c),
            _ => None,
        })
        .collect();
    let pattern = format!("^(\\./)?{}$", regex::escape(&normalized.to_string_lossy()));
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    crate::metadata::command(
        "/usr/bin/aa",
        &[
            "extract".as_ref(),
            "-i".as_ref(),
            archive.as_os_str(),
            "-d".as_ref(),
            temp.path().as_os_str(),
            "-include-regex".as_ref(),
            pattern.as_ref(),
            "-include-type".as_ref(),
            "f".as_ref(),
            "-exclude-field".as_ref(),
            "attr,xat,acl".as_ref(),
        ],
    )?;
    let path = temp.path().join(normalized);
    if !path
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(temp.path().canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("Icon archive member escapes extraction directory".into());
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

fn archive_icons(bom: &Path, archive: &Path, temporary: &Path) -> Result<Vec<PathBuf>, String> {
    if !bom.is_file() || !archive.is_file() {
        return Ok(Vec::new());
    }
    let listing = bom_listing(bom)?;
    let mut icons = Vec::new();
    for (index, entry) in listing
        .iter()
        .filter(|s| s.ends_with(".app/Contents/Info.plist"))
        .enumerate()
    {
        let entry_path = Path::new(entry);
        if entry_path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            return Err("Bundle icon archive path escapes its payload".into());
        }
        let Ok(bytes) = archive_file(archive, entry_path) else {
            continue;
        };
        let Ok(Value::Dictionary(info)) = Value::from_reader(std::io::Cursor::new(&bytes)) else {
            continue;
        };
        let Some(app) = entry_path.parent().and_then(Path::parent) else {
            continue;
        };
        let filename = info
            .get("CFBundleIconFile")
            .and_then(Value::as_string)
            .or_else(|| info.get("CFBundleIconName").and_then(Value::as_string))
            .map(str::to_owned)
            .unwrap_or_else(|| {
                app.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        if Path::new(&filename)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let filename = if Path::new(&filename).extension().is_none() {
            format!("{filename}.icns")
        } else {
            filename
        };
        let archive_icon = app.join("Contents/Resources").join(&filename);
        // Pinned Munki selects only .icns entries from package archives.
        if archive_icon.extension().and_then(|s| s.to_str()) != Some("icns") {
            continue;
        }
        let Ok(icon_bytes) = archive_file(archive, &archive_icon) else {
            continue;
        };
        let output = temporary
            .join(format!("bundle-icon-{index}"))
            .join(filename);
        std::fs::create_dir_all(output.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&output, icon_bytes).map_err(|e| e.to_string())?;
        icons.push(output);
    }
    Ok(icons)
}

/// The paths in a BOM, like `lsbom -s`.
fn bom_listing(bom: &Path) -> Result<Vec<String>, String> {
    #[cfg(unix)]
    if native() {
        return crate::icon_native::bom_paths(bom);
    }
    let listing = crate::metadata::command("/usr/bin/lsbom", &["-s".as_ref(), bom.as_os_str()])?;
    Ok(String::from_utf8_lossy(&listing)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn expand(package: &Path, expanded: &Path) -> bool {
    #[cfg(unix)]
    if native() {
        return crate::icon_native::expand(package, expanded).is_ok();
    }
    crate::metadata::command(
        "/usr/sbin/pkgutil",
        &[
            "--expand".as_ref(),
            package.as_os_str(),
            expanded.as_os_str(),
        ],
    )
    .is_ok()
}

fn flat_icons(package: &Path, temporary: &Path) -> Result<Vec<PathBuf>, String> {
    let expanded = temporary.join("expanded");
    if !expand(package, &expanded) {
        return Ok(Vec::new());
    }
    fn collect(root: &Path, temporary: &Path, icons: &mut Vec<PathBuf>) -> Result<(), String> {
        if root.join("Bom").is_file() && root.join("Payload").is_file() {
            let destination = temporary.join(format!("component-{}", icons.len()));
            icons.extend(archive_icons(
                &root.join("Bom"),
                &root.join("Payload"),
                &destination,
            )?);
        }
        for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                collect(&entry.path(), temporary, icons)?;
            }
        }
        Ok(())
    }
    let mut icons = Vec::new();
    collect(&expanded, temporary, &mut icons)?;
    Ok(icons)
}
/// Extract one application's icon; no icon is a successful empty result.
pub fn extract(package: &Path, info: &Dictionary) -> Result<Option<Vec<u8>>, String> {
    if select(Tool::Icons) == Backend::Unsupported {
        return Err("Icon extraction is only supported on macOS and Linux".into());
    }
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let png = temp.path().join("icon.png");
    let mut mount = None;
    let icon = match package
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("dmg" | "iso") => {
            mount = Some(crate::mount::Mount::new(&package.to_string_lossy())?);
            let mounted = mount.as_ref().unwrap();
            if let Some(source) = info
                .get("items_to_copy")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_dictionary)
                .and_then(|d| d.get("source_item"))
                .and_then(Value::as_string)
            {
                app_icon(Path::new(&mounted.resolve(source)?))
            } else {
                let selected =
                    if let Some(package) = info.get("package_path").and_then(Value::as_string) {
                        Some(PathBuf::from(mounted.resolve(package)?))
                    } else {
                        std::fs::read_dir(mounted.path())
                            .map_err(|e| e.to_string())?
                            .filter_map(Result::ok)
                            .map(|e| e.path())
                            .find(|p| {
                                p.extension().and_then(|s| s.to_str()).is_some_and(|s| {
                                    s.eq_ignore_ascii_case("pkg") || s.eq_ignore_ascii_case("mpkg")
                                })
                            })
                    };
                let mut icons = Vec::new();
                if let Some(package) = selected {
                    if package.is_dir() {
                        icons = bundle_icons(&package, temp.path())?;
                    } else {
                        icons = flat_icons(&package, temp.path())?;
                    }
                }
                icons.into_iter().next()
            }
        }
        Some("pkg" | "mpkg") => {
            if package.is_dir() {
                bundle_icons(package, temp.path())?.into_iter().next()
            } else {
                flat_icons(package, temp.path())?.into_iter().next()
            }
        }
        _ => return Err("Unsupported installer type for icon extraction".into()),
    };
    let result = if let Some(icon) = icon {
        convert_to_png(&icon, &png)?;
        Some(std::fs::read(png).map_err(|e| e.to_string())?)
    } else {
        None
    };
    if let Some(mut mounted) = mount {
        mounted.detach()?;
    }
    Ok(result)
}

/// Converts an icon file to PNG, choosing the image Munki prefers.
pub fn convert_to_png(source: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if select(Tool::Icons) == Backend::Apple {
        return imageio_convert_to_png(source, destination);
    }
    #[cfg(unix)]
    if native() {
        return crate::icon_native::convert_to_png(source, destination);
    }
    let _ = (source, destination);
    Err("Icon conversion is only supported on macOS and Linux".into())
}

#[cfg(target_os = "macos")]
pub(crate) fn imageio_convert_to_png(source: &Path, destination: &Path) -> Result<(), String> {
    use core_foundation_sys::{
        base::{CFRelease, CFTypeRef},
        dictionary::CFDictionaryGetValue,
        number::{kCFNumberSInt64Type, CFNumberGetValue},
        string::{kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringRef},
        url::CFURLCreateFromFileSystemRepresentation,
    };
    use std::{ffi::c_void, ptr};
    #[link(name = "ImageIO", kind = "framework")]
    extern "C" {
        fn CGImageSourceCreateWithURL(url: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
        fn CGImageSourceGetCount(source: CFTypeRef) -> usize;
        fn CGImageSourceCopyPropertiesAtIndex(
            source: CFTypeRef,
            index: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageSourceCreateImageAtIndex(
            source: CFTypeRef,
            index: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageDestinationCreateWithURL(
            url: CFTypeRef,
            kind: CFStringRef,
            count: usize,
            options: CFTypeRef,
        ) -> CFTypeRef;
        fn CGImageDestinationAddImage(
            destination: CFTypeRef,
            image: CFTypeRef,
            properties: CFTypeRef,
        );
        fn CGImageDestinationFinalize(destination: CFTypeRef) -> bool;
        static kCGImagePropertyDPIHeight: CFStringRef;
        static kCGImagePropertyPixelHeight: CFStringRef;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGImageRelease(image: CFTypeRef);
    }
    struct Owned(CFTypeRef);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }
    struct Image(CFTypeRef);
    impl Drop for Image {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CGImageRelease(self.0) }
            }
        }
    }
    // Create/Copy results have one owner. Borrowed property values are read while
    // their dictionary is alive. Native signatures use CF object pointers and
    // size_t indexes, matching the ImageIO C API.
    unsafe {
        let url = |path: &Path| {
            let bytes = path.as_os_str().as_encoded_bytes();
            Owned(
                CFURLCreateFromFileSystemRepresentation(
                    ptr::null(),
                    bytes.as_ptr(),
                    bytes.len() as isize,
                    0,
                )
                .cast(),
            )
        };
        let source_url = url(source);
        let destination_url = url(destination);
        if source_url.0.is_null() || destination_url.0.is_null() {
            return Err("Cannot create icon file URLs".into());
        }
        let source = Owned(CGImageSourceCreateWithURL(source_url.0, ptr::null()));
        if source.0.is_null() {
            return Err("Cannot read icon image".into());
        }
        let mut candidates = Vec::new();
        let mut selected = None;
        for index in 0..CGImageSourceGetCount(source.0) {
            let properties = Owned(CGImageSourceCopyPropertiesAtIndex(
                source.0,
                index,
                ptr::null(),
            ));
            if properties.0.is_null() {
                return Err("Cannot read icon image properties".into());
            }
            let number = |key: CFStringRef| {
                let value = CFDictionaryGetValue(properties.0.cast(), key.cast());
                let mut result = 0i64;
                if !value.is_null() {
                    CFNumberGetValue(
                        value.cast(),
                        kCFNumberSInt64Type,
                        (&mut result as *mut i64).cast::<c_void>(),
                    );
                }
                result
            };
            let height = number(kCGImagePropertyPixelHeight);
            let dpi = number(kCGImagePropertyDPIHeight);
            if height == 512 && dpi == 72 {
                selected = Some(index);
                break;
            }
            candidates.push((height, dpi, index));
        }
        if selected.is_none() {
            selected = candidates
                .iter()
                .find(|(h, _, _)| *h == 512)
                .map(|(_, _, i)| *i);
        }
        candidates.sort();
        if selected.is_none() {
            selected = candidates
                .iter()
                .find(|(h, d, _)| *h > 512 && *d == 72)
                .map(|(_, _, i)| *i);
        }
        if selected.is_none() {
            selected = candidates
                .iter()
                .find(|(h, _, _)| *h > 512)
                .map(|(_, _, i)| *i);
        }
        if selected.is_none() {
            selected = candidates.last().map(|(_, _, i)| *i);
        }
        let index = selected.ok_or("Icon image contains no representations")?;
        let image = Image(CGImageSourceCreateImageAtIndex(
            source.0,
            index,
            ptr::null(),
        ));
        if image.0.is_null() {
            return Err("Cannot decode icon representation".into());
        }
        let png_type = Owned(
            CFStringCreateWithCString(ptr::null(), c"public.png".as_ptr(), kCFStringEncodingUTF8)
                .cast(),
        );
        if png_type.0.is_null() {
            return Err("Cannot create PNG type".into());
        }
        let output = Owned(CGImageDestinationCreateWithURL(
            destination_url.0,
            png_type.0.cast(),
            1,
            ptr::null(),
        ));
        if output.0.is_null() {
            return Err("Cannot create PNG icon destination".into());
        }
        CGImageDestinationAddImage(output.0, image.0, ptr::null());
        if !CGImageDestinationFinalize(output.0) {
            return Err("Cannot write PNG icon".into());
        }
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Requires a macOS system icon fixture"]
    fn native_imageio_writes_png() {
        let temp = tempfile::tempdir().unwrap();
        let png = temp.path().join("icon.png");
        let fixture = temp.path().join("fixture.png");
        crate::metadata::command(
            "/usr/bin/sips",
            &[
                "-z".as_ref(),
                "512".as_ref(),
                "512".as_ref(),
                "-s".as_ref(),
                "format".as_ref(),
                "png".as_ref(),
                "/System/Applications/Calculator.app/Contents/Resources/AppIcon.icns".as_ref(),
                "--out".as_ref(),
                fixture.as_os_str(),
            ],
        )
        .unwrap();
        convert_to_png(&fixture, &png).unwrap();
        let data = std::fs::read(&png).unwrap();
        assert_eq!(&data[..8], b"\x89PNG\r\n\x1a\n");
        let dimensions = crate::metadata::command(
            "/usr/bin/sips",
            &["-g".as_ref(), "pixelHeight".as_ref(), png.as_os_str()],
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&dimensions).contains("pixelHeight: 512"));
    }
    #[test]
    #[ignore = "Requires macOS legacy package tools and a system icon fixture"]
    fn extracts_legacy_bundle_payload_icon_without_installing() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("Fixture.pkg");
        std::fs::create_dir_all(package.join("Contents")).unwrap();
        let payload = temp.path().join("payload");
        let app = payload.join("Applications/Fixture.app");
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        Value::Dictionary(Dictionary::from_iter([(
            "CFBundleIconFile",
            Value::String("Fixture.icns".into()),
        )]))
        .to_file_xml(app.join("Contents/Info.plist"))
        .unwrap();
        let icon = Path::new("/System/Applications/Calculator.app/Contents/Resources/AppIcon.icns");
        std::fs::copy(icon, app.join("Contents/Resources/Fixture.icns")).unwrap();
        crate::metadata::command(
            "/usr/bin/mkbom",
            &[
                payload.as_os_str(),
                package.join("Contents/Archive.bom").as_os_str(),
            ],
        )
        .unwrap();
        crate::metadata::command(
            "/usr/bin/ditto",
            &[
                "-c".as_ref(),
                "-z".as_ref(),
                payload.as_os_str(),
                package.join("Contents/Archive.pax.gz").as_os_str(),
            ],
        )
        .unwrap();
        let extracted = extract(&package, &Dictionary::new())
            .unwrap()
            .expect("Expected an extracted icon");
        let expected = temp.path().join("expected.png");
        convert_to_png(icon, &expected).unwrap();
        assert_eq!(extracted, std::fs::read(expected).unwrap());
        // The same BOM and icon must work when the payload is Apple Archive,
        // whose format is not recognized by tar on supported macOS releases.
        crate::metadata::command(
            "/usr/bin/aa",
            &[
                "archive".as_ref(),
                "-d".as_ref(),
                payload.as_os_str(),
                "-o".as_ref(),
                package.join("Contents/Archive.pax.gz").as_os_str(),
            ],
        )
        .unwrap();
        assert_eq!(
            extract(&package, &Dictionary::new()).unwrap(),
            Some(extracted.clone())
        );

        // Flat packages use their BOM rather than finding every app in an
        // expanded payload. Multiple applications retain the BOM ordering.
        let second = payload.join("Applications/Second.app");
        std::fs::create_dir_all(second.join("Contents/Resources")).unwrap();
        std::fs::copy(
            app.join("Contents/Info.plist"),
            second.join("Contents/Info.plist"),
        )
        .unwrap();
        std::fs::copy(icon, second.join("Contents/Resources/Fixture.icns")).unwrap();
        let flat = temp.path().join("Flat.pkg");
        crate::metadata::command(
            "/usr/bin/pkgbuild",
            &[
                "--root".as_ref(),
                payload.as_os_str(),
                "--identifier".as_ref(),
                "org.autopkg.icon-fixture".as_ref(),
                "--version".as_ref(),
                "1".as_ref(),
                flat.as_os_str(),
            ],
        )
        .unwrap();
        let flat_temp = temp.path().join("flat-icons");
        std::fs::create_dir(&flat_temp).unwrap();
        assert_eq!(flat_icons(&flat, &flat_temp).unwrap().len(), 2);
        assert_eq!(extract(&flat, &Dictionary::new()).unwrap(), Some(extracted));
    }
}
