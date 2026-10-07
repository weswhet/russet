#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    // The first byte splits the input into a signature and its content.
    let Some((&split, rest)) = data.split_first() else {
        return;
    };
    let at = (split as usize).min(rest.len());
    let _ = russet_codesign::cms::verify_detached(&rest[at..], &rest[..at]);
});
