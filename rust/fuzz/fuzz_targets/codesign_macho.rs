#![no_main]
use russet_codesign::macho;

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = macho::CodeDirectory::parse(data);
    let Ok(slices) = macho::slices(data) else {
        return;
    };
    for slice in slices.iter().take(8) {
        let Ok(blobs) = macho::blobs(slice.signature) else {
            continue;
        };
        for (_, blob) in blobs.iter().take(16) {
            if let Ok(directory) = macho::CodeDirectory::parse(blob) {
                let _ = directory.cdhash();
                let _ = directory.verify_pages(slice.image);
            }
        }
    }
});
