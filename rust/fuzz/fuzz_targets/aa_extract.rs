#![no_main]
#[path = "common.rs"]
mod common;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let out = tempfile::tempdir().unwrap();
    let _ = russet_aa::extract_stream(data, out.path(), common::limits());
    let input = common::file(data);
    let _ = russet_aa::read_member(input.path(), "a/b".as_ref(), 1 << 20);
    let _ = russet_aa::extract(input.path(), &out.path().join("file"), common::limits());
});
