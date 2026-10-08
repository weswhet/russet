# Download performance research

Investigated October 8, 2026. This is a feasibility assessment based on local code and upstream documentation, not a benchmark or implementation.

## Recommendation

Prototype an optional Russet-owned HTTP transfer engine using Tokio, reqwest, and rustls. Keep recipe/cache policy separate from transport. Compare a pooled single-stream implementation against bounded parallel range requests before choosing a default. Evaluate ripget as a prototype/reference candidate; it is not yet established as a compatible replacement.

The expected benefit is conditional: parallel ranges can help when a single connection underuses the available bandwidth. They cannot overcome an already saturated network link or an aggregate server limit. Measure real package hosts before claiming a speedup. Running independent recipes concurrently is a separate opportunity with ordering and shared-cache implications.

## Current implementation

- `rust/crates/processors/src/downloader.rs:88–143` builds a curl command for one URL; lines 191–201 execute it synchronously with `Command::output()`. The package path passes `--output` to curl at lines 825–833, so package bodies stream to a file rather than accumulate in Rust memory.
- Lines 820–824 create a fresh temporary file. Lines 873–875 retain it after failure, but there is no range scheduler or mechanism to resume from its saved offset.
- Lines 842–852 send ETag/Last-Modified conditions. Lines 436–493 handle cache change detection and Content-Length fallback. Lines 495–520 implement optional hash computation. Preserve these semantics when adding a transport.
- `rust/crates/cli/src/main.rs:458–483` runs recipes serially; `rust/crates/engine/src/lib.rs:580–618` runs recipe steps serially. Fresh curl processes do not share a connection pool across invocations.
- `rust/crates/platform/src/downloads.rs:6–49` discovers curl and honors `CURL_PATH`. `downloader.rs:116–125` accepts optional curl arguments. Arbitrary curl options cannot automatically map to a Rust HTTP client.
- `URLDownloaderPython` also preserves urllib-like behavior and uses certificate policy from `rust/crates/processors/src/download_trust.rs`, including certifi, native, and custom trust. Transport replacement must preserve that policy explicitly.
- The processors manifest and inspected lockfile do not already include reqwest, hyper, or ureq. An async HTTP engine adds dependencies and a runtime integration boundary.

These observations come from code inspection; no transfer performance tests were run. Line references describe the checkout at investigation time.

## What to borrow from aria2

The relevant subset is parallel byte-range downloads, bounded connections, retries, and resumable progress. aria2 distinguishes concurrent files from connections within a file, and uses a minimum split size to avoid splitting small files. Its documented per-server connection default is one, so benchmarks must explicitly configure splitting. See the [aria2 manual](https://aria2.github.io/manual/en/html/aria2c.html).

Russet does not need to reproduce aria2's full protocol and download-manager scope to test this idea.

## Rust candidates

| Candidate | Relevant capabilities | Assessment for Russet |
| --- | --- | --- |
| [ripget](https://docs.rs/ripget/latest/ripget/) | Parallel ranges, retries, progress, single-stream fallback, async library API. | Closest compact Rust candidate for a prototype. Its [manifest](https://github.com/sam0x17/ripget/blob/main/Cargo.toml) uses Tokio and reqwest with rustls. Audit cache validators, retry limits, custom requests, and TLS integration before adoption. |
| [parallel_downloader](https://docs.rs/crate/parallel_downloader/latest) | Range chunks, persisted resume state, retries, optional SHA-256 verification, library entry points. | Worth evaluating for resume support. Its broader CLI/daemon/TUI design and dependency surface need assessment for embedding. |
| [bytehaul](https://github.com/triwinds/bytehaul) | Range probing/fallback, resume, memory budgets, retries, progress, checksums. | Feature-rich candidate, but current upstream 0.2.7 uses vendored libcurl as its sole production transport. A Rust API does not make this an all-Rust HTTP implementation. |
| [downloader](https://docs.rs/downloader/latest/downloader/) | Concurrent downloads of different files, mirrors, validation callbacks. | Its documented focus is parallel files; that is distinct from accelerating one large package with segmented ranges. |

The retrieved bytehaul pages were inconsistent: an older docs.rs crate page and cached raw manifest showed 0.2.0 with Hyper, while the repository README and a direct fetch of its current manifest showed 0.2.7 with libcurl. Use a pinned release and inspect its actual dependency graph during evaluation. The current README and [manifest](https://github.com/triwinds/bytehaul/blob/master/Cargo.toml) support the libcurl assessment above.

These libraries demonstrate feasibility, but this research does not establish aria2-equivalent maturity or Russet compatibility. For a custom engine, reqwest provides a [reusable client with connection pooling](https://docs.rs/reqwest/latest/reqwest/struct.Client.html); Russet would own the scheduling, cache integration, and resume policy.

## Proposed first implementation

1. Introduce a transfer interface beneath the existing downloader's cache, naming, metadata, and reporting logic. Start with ordinary HTTP GET requests. Keep requests with unsupported curl options or other compatibility requirements on the existing backend.
2. Add a reusable client, bounded streaming buffers, cancellation, and download timing. Partition client reuse by TLS/proxy/auth configuration. Preserve certificate policy and redirect credential handling.
3. Experiment with four concurrent ranges for files at least 64 MiB, subject to a shared per-host limit. These are proposed starting values, not measured optimal defaults. Compare 1, 2, 4, and 8 ranges.
4. Probe actual range behavior. Require valid `206` responses, matching `Content-Range`, exact lengths, and a stable representation, preferably pinned by a strong ETag. Use identity content encoding. Cancel workers and safely restart sequentially when the server ignores ranges or the object changes. See [HTTP range and conditional request semantics](https://www.rfc-editor.org/rfc/rfc9110.html#name-range-requests).
5. Write into a temporary artifact with bounded memory and disjoint offsets. Verify complete coverage, final length, and a trusted expected checksum when one is available before publishing. A locally computed hash alone does not establish publisher integrity.
6. Add durable resume state only after range correctness is established. Bind saved segments to the resource identity and validator, rather than assuming a partially populated file is valid.

## Validation before adoption

Benchmark current Russet/curl, the proposed pooled single-stream backend, segmented Rust downloads, and explicitly configured aria2 against the same artifacts. Use repeated, interleaved trials on representative package hosts and a controlled range-capable server. Record total duration, transfer throughput, setup latency, bytes transferred, CPU, memory, and file hashes. Test both fresh downloads and unchanged cached downloads; separately measure batch completion time.

Correctness fixtures should cover ignored ranges (`200`), malformed or mismatched `206` responses, `416`, truncated bodies, changing ETags, unknown lengths, redirects, authenticated URLs, unsupported HEAD, content encoding, `429`/`503` with Retry-After, interruption, and cancellation. Run compatibility checks for request headers, curl options, trust bundles, filenames, cache validators, download flags, and supported operating systems.

Proceed with default enablement only if measured gains justify the compatibility and maintenance cost. No implementation or benchmark results are claimed by this assessment.
