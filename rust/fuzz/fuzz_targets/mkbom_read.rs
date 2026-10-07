#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(entries) = russet_mkbom::read(data) {
        for entry in entries.iter().take(64) {
            let _ = entry.lsbom_line();
        }
    }
    let _ = russet_mkbom::macho_archs(data);
});
