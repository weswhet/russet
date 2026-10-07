#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = russet_codesign::requirement::Requirement::parse(text);
    }
});
