# Fuzz targets

These [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets feed
arbitrary bytes to the parsers that read untrusted Apple formats: Apple
Archive, cpio, zip, xar, BOM, disk images, Mach-O code signatures, code
requirements, and CMS signatures. The crate is outside the Russet workspace,
so it builds with its own lock file and a nightly toolchain.

The `fuzz` job in `.github/workflows/rust.yml` runs every target for 60
seconds on each push. To run one longer, install cargo-fuzz and run it from
this folder:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly fuzz run xar_open -- -max_total_time=3600
```

Crashing inputs are saved in `artifacts/<target>/`. Add a crash's input to the
relevant crate's tests when you fix it.
