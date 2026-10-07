#![no_main]
#[path = "common.rs"]
mod common;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let input = common::file(data);
    let _ = russet_hdiutil::image_info(input.path());
    let out = tempfile::tempdir().unwrap();
    let _ = russet_hdiutil::extract(input.path(), out.path(), common::limits());
});
