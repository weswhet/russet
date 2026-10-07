#![no_main]
#[path = "common.rs"]
mod common;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let input = common::file(data);
    let Ok(mut archive) = russet_xar::Archive::open(input.path()) else {
        return;
    };
    let _ = archive.signatures();
    let paths: Vec<String> = archive
        .entries()
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    for path in paths.iter().take(16) {
        let _ = archive.read(path, 1 << 20);
    }
    let out = tempfile::tempdir().unwrap();
    let _ = archive.extract(out.path(), common::limits(), |_| false);
});
