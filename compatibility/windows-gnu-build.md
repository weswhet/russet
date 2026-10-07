# Windows GNU cross-build validation

On October 6, 2026, the development CLI built successfully for
`x86_64-pc-windows-gnu`. A final incremental build included the frozen source
changes to processor receipts, Chocolatey null handling, and default cache resolution.

| Item | Observed value |
| --- | --- |
| Artifact | `rust/dist/windows-gnu/autopkg-rs.exe` |
| SHA-256 | `25692293ca99354230ffd391c538317afb28b6650c30c23aa16c7afd3f0c0042` |
| Format | PE32+, x86-64, Windows console executable |
| Rust host | `aarch64-unknown-linux-gnu` |
| Rust compiler | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Cross-compiler | Debian MinGW GCC 12-win32 |
| Container | `rust:1.97-bookworm`, `linux/arm64` |
| Image manifest digest | `sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97` |
| Cargo.lock SHA-256 | `ba158076fd27fe38c0f9dc0e4ee0dc6d9a34fbbc3ad8df89aca7941571acc659` |

The build used the locked dependencies and normal release optimization:

```sh
cargo build --locked --release --target x86_64-pc-windows-gnu -p autopkg-rs
```

The container installed `gcc-mingw-w64-x86-64` and the Rust Windows GNU target.
It made no host toolchain changes. The clean native ARM64-hosted build took
1 minute 29 seconds; the final incremental build took 18.17 seconds.

`file` and MinGW `objdump -p` confirmed the executable format and imports.
The imported DLLs are `kernel32.dll`, `bcryptprimitives.dll`, `msvcrt.dll`,
`ntdll.dll`, `USERENV.dll`, `WS2_32.dll`, and
`api-ms-win-core-synch-l1-2-0.dll`. No separate MinGW runtime DLL appeared
in the import table. The raw PE inspection is saved beside the executable as
`pe-headers.txt`.

The complete workspace, including test and helper targets, also passed this
Windows-target compile check in 1 minute 16 seconds. The final incremental check
including cache resolution passed in 1.38 seconds:

```sh
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
```

The packaging script, which `cargo xtask package` has since replaced, produced
`rust/dist/autopkg-rs-development-x86_64-pc-windows-gnu.zip`. ZIP integrity
and equality of its executable bytes to the built artifact were verified.
The archive SHA-256 is
`baa7c6675e38019fd45cb46ef126011defcab677f8c33202006a575be7e0c33e`.

Two earlier attempts using an emulated `linux/amd64` compiler container failed
inside GCC while compiling the bundled LZMA C sources. Both normal `-O3` and a
container-local `-O2` retry encountered a `cc1` segmentation fault. Building
with an ARM64 host compiler targeting Windows x64 resolved the failure without
source changes or reduced optimization.

This validates cross-compilation and binary format only. The executable has
not run on Windows or Wine in this check. Native Windows processor tests,
MSVC builds, installation, upgrade, rollback, signatures, and clean-machine
execution without Python remain release gates. This GNU artifact is a
development build and does not establish an older Windows support baseline.
