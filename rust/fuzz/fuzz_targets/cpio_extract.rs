#![no_main]
#[path = "common.rs"]
mod common;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let input = common::file(data);
    let out = tempfile::tempdir().unwrap();
    let _ = russet_ditto::extract_cpio(input.path(), out.path(), common::limits());
    let _ = russet_ditto::read_cpio_member(input.path(), "a/b".as_ref(), 1 << 20);
});
