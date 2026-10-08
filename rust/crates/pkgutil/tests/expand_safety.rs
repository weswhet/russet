//! Packages crafted to make expanding or flattening write the wrong place.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// A gzip-compressed cpio archive of `folder`, as a `Scripts` member.
fn scripts_archive(folder: &Path) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    russet_ditto::write_tree(folder, encoder, |_, m| russet_ditto::Header {
        mode: m.mode(),
        uid: m.uid(),
        gid: m.gid(),
        mtime: 0,
        ino: 0,
        nlink: 1,
    })
    .unwrap()
    .finish()
    .unwrap()
}

/// A xar archive with a `Scripts` file and a `.Scripts.russet` symlink to
/// `target`, the name expansion once unpacked scripts into.
fn package_with_symlink(scripts: &[u8], target: &Path) -> Vec<u8> {
    let toc = format!(
        "<xar><toc><file id=\"1\"><name>Scripts</name><type>file</type><data>\
         <offset>0</offset><length>{0}</length><size>{0}</size></data></file>\
         <file id=\"2\"><name>.Scripts.russet</name><type>symlink</type>\
         <link>{1}</link></file></toc></xar>",
        scripts.len(),
        target.display()
    );
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(toc.as_bytes()).unwrap();
    let compressed = encoder.finish().unwrap();
    let mut out = b"xar!".to_vec();
    out.extend_from_slice(&28u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_be_bytes());
    out.extend_from_slice(&(toc.len() as u64).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend(compressed);
    out.extend_from_slice(scripts);
    out
}

#[test]
fn scripts_never_unpack_through_a_packaged_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("victim"), "original\n").unwrap();
    let scripts = temp.path().join("scripts");
    fs::create_dir(&scripts).unwrap();
    fs::write(scripts.join("victim"), "overwritten\n").unwrap();
    let package = temp.path().join("escape.pkg");
    fs::write(
        &package,
        package_with_symlink(&scripts_archive(&scripts), &outside),
    )
    .unwrap();

    let expanded = temp.path().join("expanded");
    // Expansion may succeed or fail, but never writes outside.
    let _ = russet_pkgutil::expand(&package, &expanded, Default::default());
    assert_eq!(
        fs::read_to_string(outside.join("victim")).unwrap(),
        "original\n"
    );
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
}

/// `a.pkg/b.pkg/Scripts` and `a.pkg-b.pkg/Scripts` keep their own scripts
/// through a flatten and expand.
#[test]
fn flatten_keeps_similarly_named_components_apart() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    for (component, script) in [("a.pkg/b.pkg", "first\n"), ("a.pkg-b.pkg", "second\n")] {
        let folder = input.join(component).join("Scripts");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("postinstall"), script).unwrap();
    }
    let package = temp.path().join("out.pkg");
    russet_pkgutil::flatten(&input, &package).unwrap();
    let output = temp.path().join("expanded");
    russet_pkgutil::expand(&package, &output, Default::default()).unwrap();
    for (component, script) in [("a.pkg/b.pkg", "first\n"), ("a.pkg-b.pkg", "second\n")] {
        assert_eq!(
            fs::read_to_string(output.join(component).join("Scripts/postinstall")).unwrap(),
            script,
            "{component}"
        );
    }
}
