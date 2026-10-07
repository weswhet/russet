//! Native replacement for the `hdiutil` operations Russet uses.
//!
//! Linux can't mount Apple disk images, so [`extract`] reads the image and
//! writes each volume's contents into a folder, the way `hdiutil attach`
//! makes them visible on macOS. [`image_info`] reports what
//! `hdiutil imageinfo` would, such as the image `Format`.
//!
//! Supported: UDIF images (`.dmg`) with zlib (`UDZO`), bzip2 (`UDBZ`), LZFSE
//! (`ULFO`), LZMA (`ULMO`), ADC (`UDCO`), or uncompressed (`UDRO`) data,
//! containing HFS+, HFSX, or APFS volumes. Encrypted images and ISO 9660
//! images aren't supported yet and fail with a clear error.
#![forbid(unsafe_code)]

#[cfg(unix)]
mod apfs_volume;
#[cfg(unix)]
mod hfs_volume;
#[cfg(unix)]
mod image;

#[cfg(unix)]
pub use image::{extract, image_info, Extraction, ImageInfo};

#[cfg(all(test, target_os = "macos"))]
mod apple_tests;

#[cfg(all(test, unix))]
mod fixture_tests;
