# Native download implementation plan

Proposed October 8, 2026. Phases 1 through 4 are implemented; phase 5 (durable resume) is not. The sections after the status describe the original proposal.

## Status

Updated October 9, 2026.

| Phase | Status |
| --- | --- |
| 1. Capture compatibility | Done. See the [curl option inventory](download-curl-inventory.md) and the generated-argument baseline test in `url_downloader.rs`. |
| 2. Extract the backend boundary | Done. Every download goes through `download_transport::run`. |
| 3. Native single-stream | Done, and on by default. See the deviations below. |
| 4. Parallel chunks | Done, for resources of 64 MiB or more with a strong ETag. |
| 5. Durable resume | Not started. A failed transfer leaves no reusable partial state. |
| 6. Benchmark and expand coverage | Live-recipe comparison only; no controlled benchmark yet. |
| 7. `auto` by default | Done early, at the user's request; see below. |

### What shipped

- `download_transport/options.rs` inspects the exact curl command the downloader built. It admits the generated options, `--cacert`, `--capath`, and the effective-URL `--write-out` for `URLDownloaderPython`, and these recipe options: `--location`/`-L`, `--fail`/`-f`, `--silent`/`-s`, `--show-error`/`-S`, `--no-buffer`/`-N`, `--header`/`-H`, `--user-agent`/`-A`, and `--referer`/`-e`, including short-option groups and attached values. Everything else, including `--name=value` spellings, file-sourced headers, unsupported header names, removal of `Accept`, automatic referers, multiple URLs, non-HTTP(S) URLs, URLs with credentials or glob characters, proxy variables, `CURL_CA_BUNDLE`, `CURL_SSL_BACKEND`, `SSL_CERT_DIR` on Windows, curl configuration files, and an explicit `CURL_PATH`, runs the original curl command unchanged. The decision, trust loading, and client construction happen before any network activity; a native request is never replayed through curl.
- `download_transport/native.rs` owns one multi-threaded Tokio runtime and a pool of reqwest clients over rustls with the ring provider, keyed by trust roots and low-speed timeout. It follows redirects manually to reproduce curl's header dump (`http_redirected`, final headers, HTTP/2 status lines without a reason phrase), drops recipe `Authorization` and `Cookie` headers on cross-origin redirects as curl does, applies `--fail`, `--speed-time`, curl's 50-redirect limit, and identity encoding, and reports failures with curl's exit codes and messages so `curl::check_exit` handles both backends identically.
- Trust follows each processor variant. `URLDownloaderPython` uses the generated bundle and CA path. `URLDownloader` follows the curl tool: the `SSL_CERT_FILE` the curl child would see (the certifi bundle on macOS) or the distribution bundle, plus any `SSL_CERT_DIR` directories, as curl adds them to its bundle; with neither variable set, Linux and Windows use the platform verifier. GitHub's Ubuntu images set `SSL_CERT_DIR`.
- The default `User-Agent` is the one the fallback curl sends, read once from `curl --version` (for example, `curl/8.7.1`), so vendor servers that check the user agent see no change. A recipe user agent or `User-Agent` header replaces it.
- `download_transport/chunks.rs` splits a `200` response with a known length of at least 64 MiB, a strong ETag, and no content coding into 8 MiB chunks. The original response keeps streaming from the start while up to three HTTP/1.1 range workers take chunks from the end, with `Range` and `If-Range` on every request, positional writes, exact `206`, `Content-Range`, ETag, and length validation, and caps of eight range requests overall and three per origin. On any anomaly, the workers are cancelled and joined and the original response takes over the remaining chunks; if it has already stopped, the transfer restarts once as a single stream.
- `AUTOPKG_RS_DEBUG=1` prints the backend chosen, the redacted fallback reason, the transfer mode, and the elapsed time.

### Deviations from the proposal

- There is one preference, `UseRussetDownloader` (default `true`), instead of `DOWNLOAD_BACKEND` modes. `true` behaves like the proposed `auto`; `false` behaves like `curl`. There is no strict `native` mode. The preference is read like `CURL_PATH`: from the recipe environment (preferences, `AUTOPKG_` variables, and `--key`), then the macOS preference domain.
- The chunk size, minimum size, and worker count are constants, not settings.
- The native default shipped before a controlled benchmark, because the user requested it. The release gates in [Benchmark and release criteria](#benchmark-and-release-criteria) still apply to further expansion.
- A TLS handshake that rustls can't complete (for example, a server that offers only TLS 1.0 or CBC cipher suites) fails instead of falling back, because the decision is made before network activity. Set `UseRussetDownloader` to `false` for such servers.

### Tests

`download_transport/tests/` holds deterministic local HTTP and HTTPS fixtures. `parity.rs` runs each scenario through both processors with curl and with the native engine and requires identical processor outputs, errors, cached files, metadata, and server-side requests: conditional caching and `304`, header capture with duplicates and custom reason phrases, request header overrides and removal, redirect chains with cross-origin credential stripping, `404`/`500`/`403` with `--fail`, short bodies, `Content-Disposition` and `Location` prefetch, and verified `curl_opts`. `ranges.rs` covers parallel reassembly, ignored ranges, changed ETags, mismatched `Content-Range`, a late worker failure that forces a restart, a short original stream, small files, weak validators, and TLS trust for both processor variants. `options.rs` covers admission, redacted fallback reasons, and the preferences.

## Outcome and compatibility contract

Add a Russet-owned Rust HTTP download engine with pooled connections, parallel chunks, and durable resume. Keep existing recipes working, including recipes that supply arbitrary `curl_opts`. Build on the [download performance research](download-performance-research.md), and measure improvements before changing the default backend.

Preserve curl compatibility at the recipe boundary through three proposed modes:

| Mode | Behavior |
| --- | --- |
| `auto` | Use native transfer only when the entire invocation and its environment have verified equivalent behavior. Otherwise execute the existing configured curl command with its original arguments. |
| `native` | Require native support. Reject unsupported or ambiguous semantics before network activity, with a specific reason and a suggestion to use `auto` or `curl`. |
| `curl` | Execute the existing curl backend and preserve its behavior. |

Universal native support for every current and future curl flag is outside this plan. curl covers protocols, authentication systems, TLS backends, configuration files, and command behaviors that a Rust HTTP client does not reproduce automatically. The compatibility guarantee is that `auto` retains access to everything the user's configured curl currently supports; it does not make unsupported curl features native.

Never silently discard an option, approximate a required behavior, or reinterpret a recipe merely to enable acceleration. A native-only requirement for all curl flags would need a separate, substantially larger compatibility project.

## Preserve the existing integration

Keep recipe policy in [url_downloader.rs](../crates/processors/src/processors/url_downloader.rs): cache decisions, download naming, output variables, metadata, optional hashing, and reporting.

The current command builder assembles generated curl arguments, operation arguments, request headers, recipe options, and certificate arguments in order. Later call sites append cache conditionals and Python-specific write-out arguments; inspect the complete final invocation. Each `curl_opts` string is one argument, without shell splitting. `URLDownloader` accepts `curl_opts`; `URLDownloaderPython` follows a different path and does not append those options. Preserve that distinction.

Continue honoring [curl discovery and `CURL_PATH`](../crates/platform/src/downloads.rs) and [certificate policy](../crates/processors/src/processors/url_downloader/trust.rs). An explicit `CURL_PATH`, including a wrapper executable, selects curl in `auto` until its behavior can be proven equivalent; recognizing the filename alone is insufficient.

Use the existing processor dispatch for both aliases; support both recipe execution and standalone processor runs without introducing another processor name. Keep recipe and processor execution sequential in this project. Parallelism is inside one eligible transfer, with shared limits that remain safe if recipe concurrency is added later.

## Proposed module boundaries

All new paths below are proposals. Keep them inside the processors crate initially; extract a library only if an independent consumer emerges. Use `download_transport` to avoid colliding with the `url_getter.rs` module for text fetching and the other download processors.

| File or area | Responsibility |
| --- | --- |
| `crates/processors/src/processors/url_downloader.rs` | Existing processor behavior; create requests and consume transfer results. |
| `crates/processors/src/download_transport/mod.rs` | Transfer interface, backend selection, shared runtime and client ownership. |
| `crates/processors/src/download_transport/curl.rs` | Existing curl invocation and response adaptation, preserved during extraction. |
| `crates/processors/src/download_transport/options.rs` | Ordered invocation inspection, semantic registry, eligibility decisions. |
| `crates/processors/src/download_transport/native.rs` | Native single-stream HTTP transfer, redirects, bounded streaming. |
| `crates/processors/src/download_transport/chunks.rs` | Range eligibility, scheduling, validation, retry budgets. |
| `crates/processors/src/download_transport/resume.rs` | Locked partial artifacts, durable sidecars, recovery and publication. |
| `crates/processors/src/processors/url_downloader/trust.rs` | Existing trust policy plus explicit native adapter. |
| `crates/processors/src/lib.rs` | Module registration and the smallest necessary execution-context changes. |
| `crates/processors/Cargo.toml`, `Cargo.lock` | Pinned transport/runtime dependencies and selected features. |
| `crates/processors/tests/` | Differential fixtures, local HTTP/TLS servers, crash and resume tests. |
| `docs/download-performance-research.md` | Append measured benchmark findings after implementation. |

Define a `TransferRequest`, `TransferOutcome`, and typed failure categories. Keep the original executable, ordered `OsString` arguments, inherited environment, working directory, and trust-bundle lifetime available for curl fallback. A separate normalized request serves native execution; never reconstruct fallback arguments from that representation.

A transfer outcome carries final response headers, effective URL, status, byte counts, and staged artifact ownership. Separate whole-resource metadata from chunk headers: a partial response Content-Length must never become the cache comparison size. Existing processor code remains responsible for publishing recipe outputs. Adapt error categories to existing processor failure behavior and test the adaptation.

Use Tokio, reqwest, and rustls as the initial prototype stack. Review pinned versions, feature flags, supported platforms, and the workspace's Rust version before adding dependencies. Existing download libraries may supply ideas or components only if they pass the same contract and tests.

Own one reusable runtime and client pool at a stable execution boundary. Avoid a runtime or client per chunk, nested blocking runtimes, and leaked worker tasks. Partition clients by trust roots, proxy policy, client identity, and other connection-affecting settings; isolate cookies and credentials between recipes.

## Interpret curl semantics conservatively

Treat compatibility as an invocation-level decision. The [curl manual](https://curl.se/docs/manpage.html) is the reference for option syntax and behavior; pin the supported curl versions and builds used in comparison tests.

1. Capture the exact command produced today before executing it, including generated defaults and trailing certificate arguments.
2. Inspect arguments in order, retaining duplicates and provenance. Handle long options, supported `--name=value` forms, short options with attached arguments, short-option groups, negation, and `--` according to the tested curl version.
3. Model each option's actual repeat behavior: replacement, accumulation, toggling, or transfer-local scope. Do not assume every option uses its last value.
4. Evaluate combinations and interactions with generated options, such as user headers overriding conditional headers or options redirecting output away from the generated temporary file.
5. Include implicit configuration: default curl configuration files, environment variables, proxy exclusions, certificate overrides, locale-sensitive inputs, and filesystem inputs where relevant.
6. Return `native_chunkable`, `native_sequential`, or `curl_required`, with a redacted reason code. In strict mode, `curl_required` becomes an actionable preflight error.

In `auto`, unknown options, invalid-looking options, unsupported syntax, or unresolved interactions go to the original curl invocation. Let curl perform its existing validation; do not pre-reject a newer valid flag because Russet's registry does not know it.

Default and explicit curl configuration files, `--config`, expansion mechanisms, multiple URLs, URL globbing, `--next`, stdin-dependent options, and file-driven header/body options initially require curl. Do not read an incomplete subset of a configuration file and claim parity. Do not inject `--disable` into fallback to simplify native eligibility.

Discover implicit configuration using the supported curl builds' rules. If configuration presence or applicability cannot be established, select curl. Native eligibility requires either a verified configuration interpretation or evidence that no relevant implicit configuration applies.

The registry records parsing rules, semantic effects, interactions, redirect handling, retry safety, chunk eligibility, supported platforms, and fixture identifiers. Mark a combination native only after differential tests pass. A similarly named [reqwest client setting](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html) is an implementation tool, not evidence of equivalent behavior.

### Initial coverage and expansion targets

These are targets for implementation, not a list of flags already supported natively. Every unverified row or combination uses curl.

| Category | Initial native target | Deferred or conservative behavior |
| --- | --- | --- |
| Basic transfer | Generated HTTP(S) GET, output staging, header capture, error reporting | Additional URLs, non-HTTP protocols, output redirection overrides use curl. |
| Headers | Verified `request_headers`, `--header`/`-H`, user agent, referer | Preserve duplicates, empty/removal semantics, sensitive redirect rules; file inputs use curl initially. |
| Redirects | Generated location behavior and verified redirect limits | Protocol changes, trusted redirects, custom redirect/method interactions use curl until covered. |
| Authentication | Explicitly tested Basic/Bearer flows and credential scoping | Digest, NTLM, Negotiate, netrc, signing, challenge interactions use curl initially. |
| TLS and trust | Existing downloader trust modes with equivalent root and hostname validation | Unsupported CA directory semantics, TLS knobs, hardware identities, pinning, revocation differences use curl. |
| Proxies and DNS | Direct connections first; verified environment and explicit proxy behavior next | Unknown proxy environment, SOCKS variants, resolver and interface controls use curl. |
| Timing and retries | Existing generated low-speed behavior, then tested timeout/retry/rate options | Any unimplemented timing interaction uses curl; budgets apply across chunks. |
| Encoding | Identity representation with transparent decoding disabled | `--compressed`, `--raw`, explicit encoding intent use curl until exact sequential behavior is covered. |
| Methods and bodies | Ordinary GET; separate tested metadata requests | Uploads, forms, bodies, arbitrary methods use curl initially. |
| Ranges and continuation | Russet-generated chunks for an otherwise ordinary GET | Explicit `--range`, `--continue-at`, or corresponding headers use curl initially, then verified sequential handling only. |
| Cookies and output extras | Adapt Russet-generated header capture and Python status write-out to typed results | Recipe-supplied cookie jars, write-out, traces, special output files, and side effects use curl initially. |

Never automatically combine explicit range or continuation intent with Russet chunking. Preserve the recipe's requested bytes, method, representation, and destination. Authentication or cookie flows that mutate session state are ineligible for chunks until concurrency is proven safe.

Select the backend before network or externally visible file side effects. Do not run a native request and then replay it through curl after an ambiguous failure. Runtime sequential restarts are permitted only within the verified safe GET path, with bounded restart counts and the same security policy.

## Implement native single-stream transfers first

Use a bounded buffer to stream into a staged file. Keep memory use proportional to active buffers, not package size. Reuse eligible connections across downloads, and expose cancellation to the synchronous processor boundary.

Explicitly configure redirects, proxy discovery, timeout behavior, decompression, TLS roots, and credential forwarding. Disable library defaults that conflict with the recorded curl contract. Test certificate policy separately for `URLDownloader` and `URLDownloaderPython`.

Preserve existing conditional-cache behavior. Send cache validators only for a complete usable cached artifact. A `304` may reuse that artifact; a partial file or sidecar is never a valid cache hit. When cache metadata exists without a complete file, recover with a bounded unconditional safe GET.

Preserve filename selection, content-disposition behavior, download-changed reporting, hashes, timestamps, and platform metadata. Match the current processor's handling of HTTP errors and headers across redirects, including cases where behavior differs from generic HTTP-client defaults.

Acceptance: curl extraction causes no behavior change; native single-stream fixtures match the baseline for every admitted request; unsupported combinations deterministically select curl before execution.

## Add parallel chunks

Use HTTP range and conditional request rules as the protocol foundation. In particular, validate returned ranges and pin the representation across requests; see [RFC 9110 range requests](https://www.rfc-editor.org/rfc/rfc9110.html#name-range-requests).

### Establish eligibility

1. Start only from a verified safe HTTP(S) GET without a request body or conflicting range, encoding, method, or conditional intent.
2. Request identity encoding and disable automatic decompression only when doing so preserves the admitted request's semantics. An explicit incompatible encoding preference prevents chunking.
3. Probe actual range behavior with a small GET range; do not trust `Accept-Ranges` or require HEAD support. Count probes against all limits.
4. Require a valid `206`, an exact bounded `Content-Range`, a known total length, identity encoding, and a strong ETag. Weak ETags and Last-Modified alone do not qualify initially.
5. Bind workers to the validated representation and redirect policy. If eligibility fails, use a safe sequential transfer without reusing uncertain bytes.

Avoid adding a mandatory HEAD or range probe to every small download or complete-cache validation; probe only when available metadata or transfer policy makes chunking worthwhile. Do not split small files or unknown-length resources. Keep resources without a strong validator sequential initially, including across restarts.

### Schedule and validate

Partition the object into nonoverlapping fixed ranges, including a smaller final range. Start with four workers as an experiment. Prefer HTTP/2 stream reuse where available; distinguish concurrent requests from physical connections when enforcing limits.

Use shared semaphores for active requests and connections, with global and per-origin caps. Account for probes, retries, redirects, and connection establishment. When a redirect changes origin, release or transfer permits without exceeding either origin's limits or causing deadlock.

Send `Range` and `If-Range` with the pinned strong ETag on every chunk. Accept a chunk only when status, exact range bounds, total size, validator consistency, encoding, and received length match expectations. Reject oversized bodies before they cross assigned boundaries.

Write to disjoint offsets using platform-appropriate positional I/O. Do not share a seek cursor among workers. Bound queued buffers, active chunks, header sizes, and total artifact size; check arithmetic for overflow before allocation or writes.

On ignored ranges (`200`), invalid ranges, changed validators, `416`, or representation mismatch, cancel and join all workers before changing file state. Invalidate unsafe partial data. Permit at most a configured bounded safe restart; repeated instability returns a clear error.

Do not mix versions or overwrite a good cached package with a partially successful transfer. Require complete contiguous coverage, final length, and any recipe-provided expected checksum before publication. A locally calculated hash detects local corruption but does not prove publisher authenticity.

### Bound retries and server load

Retry eligible transient failures with jittered backoff and bounded attempts. Honor applicable `Retry-After` within the overall deadline. Apply per-transfer elapsed-time, retry, byte-rate, and request-rate limits across all workers, including probes and restarts.

A recipe's rate limit must not multiply by worker count. Model low-speed detection and timeout scopes explicitly; route combinations to curl when equivalent behavior is unproven. Use a shared rate controller rather than independent per-worker limits.

On cancellation, stop scheduling, cancel requests, join workers, and persist only completed durable chunks. Never replay non-idempotent or otherwise unsafe operations as a retry or backend fallback.

Acceptance: every completed parallel artifact matches the known fixture hash; malformed or changing responses cannot publish mixed content; concurrency, memory, retry, and rate limits hold under failure.

## Add persistent resume and safe publication

Use a stable partial file and versioned sidecar in the destination filesystem, separate from the complete cache artifact. Define an explicit state machine: `downloading`, `ready_to_publish`, and `published`/cleanup. Document recovery for each state before implementation.

### Identify and protect partial state

- Key state by processor mode, destination, URL/request identity, representation-affecting headers, credential context, and transport policy. Include identity changes caused by redirects or authenticated sessions conservatively.
- Avoid plaintext URLs with signed queries, passwords, tokens, cookies, or authorization headers in sidecars and logs. Use a locally keyed digest for sensitive identity inputs; invalidate saved state if the key or relevant context is unavailable.
- Store schema version, total length, strong validator, chunk layout, completion bitmap, completed-chunk hashes, and publication state. Bound field sizes and chunk counts before parsing or allocating.
- If remote validators contain sensitive values, protect them with local encryption or decline persistence. Keep state files private and avoid logging validator contents.
- Acquire an OS-backed exclusive lock before inspecting or mutating state. Two processes targeting the same artifact must serialize or fail cleanly; a PID file alone is insufficient.
- Use safe file opens and validate file type, ownership, containment, and symlink behavior. Never follow sidecar-provided paths or allow a malformed manifest to choose arbitrary offsets or allocations.

### Commit progress durably

1. Download and validate a complete chunk into its assigned range.
2. Flush and sync its data according to the platform durability contract.
3. Compute and record the completed chunk's hash; update the completion map only after its data is durable.
4. Write the new sidecar to a temporary file, sync it, atomically replace the old sidecar, and sync the containing directory where supported.
5. Allow batched commits for performance, but treat all uncommitted chunks as incomplete after a crash.

Never infer completion from file length or preallocation. An interrupted current chunk is downloaded again in full. Rehash each saved completed chunk before reuse; corrupt or missing chunks become incomplete.

Before resuming, revalidate request identity, total length, strong ETag, and representation through a bounded conditional range probe. A changed or unavailable validator invalidates reuse. A `304` alone does not validate a partial artifact. Expired credentials or signed URLs do not justify merging new bytes with old state. Return an actionable error or invalidate state; do not invent a token-refresh mechanism.

### Publish and recover

After all chunks are durable, verify coverage and the full artifact hash when required. Prepare existing cache metadata, timestamps, and platform xattrs before replacement where possible. Preserve the previous complete artifact until the replacement is ready.

The existing publication path removes the destination before persisting the temporary file, then writes metadata. Preserve that legacy curl path during extraction; implement improved native publication as a separately tested change, covering the existing `.info.json` metadata and Unix xattrs. Use atomic replacement on the destination filesystem, followed by required directory synchronization. Where artifact and metadata require separate writes, record a recoverable publication transaction and reconcile it on startup; do not claim a multi-file rename is atomic.

Delete the sidecar only after publication and metadata recovery are durable. A crash after replacement but before cleanup must recognize and verify the published file rather than redownload or erase it. Failed transfers leave the last complete cache usable.

Define bounded age and disk-space cleanup for abandoned partials, skipping active locks. Handle disk-full and permission failures without marking unwritten chunks complete. Document platform-specific durability limits and test recovery on supported operating systems.

Acceptance: interruption saves completed verified chunks, restart transfers only missing chunks plus validation overhead, and every injected crash boundary recovers to a verified complete artifact or a resumable partial.

## Proposed configuration and diagnostics

These names and values are design proposals. Resolve placement and precedence against existing configuration conventions before exposing them; recipe semantics always constrain acceleration.

| Proposed setting | Initial value or policy |
| --- | --- |
| `DOWNLOAD_BACKEND` | `curl` during rollout; `auto` only after acceptance gates. |
| `DOWNLOAD_PARALLEL_CHUNKS` | `1` initially; experiment with `4` after correctness tests. |
| `DOWNLOAD_CHUNK_SIZE` | Experiment with `8 MiB`, checked against size/count limits. |
| `DOWNLOAD_MIN_PARALLEL_SIZE` | Experiment with `64 MiB`; measure before selecting a default. |
| `DOWNLOAD_MAX_REQUESTS` | Proposed shared cap of `8`, including probes and retries. |
| `DOWNLOAD_MAX_REQUESTS_PER_ORIGIN` | Proposed cap of `4`; independently cap physical connections. |
| `DOWNLOAD_RESUME` | Disabled until durability tests pass; applies only to eligible native transfers. |

Validate settings before execution and include them in bounded resource accounting. Document global and recipe override precedence, invalid values, and how to return to curl immediately.

Use the existing `AUTOPKG_RS_DEBUG=1` development diagnostics for backend choice, redacted fallback reasons, chunk counts, reused bytes, retry counts, timing, and publication recovery. Redact URL queries, credentials, headers, and sensitive paths. Normal output should retain existing recipe-facing behavior.

## Deliver in phases

1. **Capture compatibility.** Inventory curl options in a sanitized representative recipe corpus; capture complete generated commands, configuration/environment dependencies, curl versions/features, and both downloader variants. Add deterministic fixtures for existing outputs and cache metadata. Exit when the baseline is reproducible.
2. **Extract the backend boundary.** Move curl execution behind the interface without altering arguments or trust lifetimes. Add the registry and selector with all requests still on curl. Exit when baseline fixtures and existing processor tests pass unchanged.
3. **Ship opt-in native single-stream.** Add the runtime, client pool, trust adapter, and first verified semantic groups. Expand groups only with differential tests. Exit when strict errors are preflight-only and all admitted requests match baseline behavior.
4. **Add opt-in chunks.** Implement eligibility, bounded scheduling, positional writes, range validation, cancellation, and shared retry budgets. Exit when protocol fault fixtures and resource-limit tests pass.
5. **Add opt-in durable resume.** Implement locked state, validation, durable commits, recovery, and atomic publication. Exit when restart tests prove missing-chunk reuse and crash/disk-failure tests preserve cache correctness.
6. **Benchmark and expand coverage.** Measure native sequential and chunked modes against curl and aria2; expand options based on actual recipe frequency. Publish native/fallback coverage and measured results.
7. **Consider `auto` by default.** Enable only after compatibility and recovery gates pass on supported platforms and measured benefits justify the maintenance cost. Keep explicit curl rollback and unknown-option fallback permanently available.

Each phase should be independently reviewable. Keep production behavior on curl until an opt-in mode and its tests are complete. Do not combine a default switch with the initial transport rewrite.

## Validation matrix

Use deterministic local Rust HTTP/TLS fixtures for correctness, extending the existing inline downloader TCP fixtures with a concurrent harness, and separate opt-in public-host benchmarks. Preserve the retained-temporary-file failure test and add missing curl-option propagation coverage. Never depend on a live package host for the core test suite.

| Area | Required cases and assertions |
| --- | --- |
| Option parsing | Repetition, negation, aliases, short groups, attached values, `--`, malformed/missing values, unknown/future flags; original argv unchanged on fallback. |
| Invocation interactions | Generated options versus recipe overrides, multiple URLs, config files, wrapper `CURL_PATH`, environment proxy/trust changes; eligibility decided before network. |
| Differential behavior | Compare request captures, artifact bytes, headers, redirects, output variables, cache decisions, metadata, errors, and filesystem side effects with supported curl builds. |
| Processor variants | `URLDownloader` and `URLDownloaderPython` input and trust differences, certifi/native/custom roots, naming, hash, changed flags, and cached downloads. |
| Network security | Authenticated redirects on same/different origins, downgrade rules, cookie isolation, Basic/Bearer flows, proxy auth and exclusions, invalid/expired TLS, custom roots. |
| Range protocol | Ignored ranges, malformed/overlapping ranges, short/long bodies, unknown size, empty files, final partial chunk, `416`, ETag changes, weak validators, compressed responses, rejected HEAD. |
| Cache | Complete-cache `304`, orphaned metadata, partial-state conditional requests, unchanged/changed resources, filenames and xattrs after publication. |
| Retry and limits | Reset connections, timeouts, `429`/`503`, `Retry-After`, cancellation, aggregate deadlines/rates, bounded restarts, redirects across origins, no unsafe replay. |
| Persistence | Process kill mid-chunk and after each sync/rename boundary, corrupted chunk or manifest, stale validator, schema mismatch, concurrent processes, expired credentials. |
| Filesystem failures | Disk full, permissions, interrupted metadata writes, symlinks, hostile paths, oversized fields, overflowing offsets, cleanup races, publication recovery. |
| Resource use | Large sparse fixtures, bounded memory, request/connection caps, worker cleanup, file descriptor leaks, slow consumers, and no per-package-size buffering. |

Use property tests or fuzzing for the option inspector, range parser, and sidecar decoder where malformed input has complex state interactions. Keep fixture inputs sanitized and never record production credentials.

Run the existing processor suite plus focused new integration tests per phase. From `rust/`, use the established checks:

```sh
rtk cargo fmt --all -- --check
rtk cargo clippy --workspace --all-targets --locked -- -D warnings
rtk cargo test --workspace --exclude xtask --locked
rtk cargo test --package xtask --locked
```

Validate macOS arm64 and x86_64, Windows MSVC x86_64, and Linux x86_64 in the existing CI matrix, including packaging/install checks. Test locking and replacement with each platform's APIs. Record skipped platform checks explicitly. Update preference documentation and compatibility references when settings ship; do not tighten the existing recipe schema to reject curl options.

## Benchmark and release criteria

Compare current curl, pooled native single-stream, native with 1/2/4/8 workers, and aria2 configured with equivalent connection and split limits. Use identical artifacts and verify hashes after every successful trial.

Measure controlled single-connection throttling, unconstrained LAN, high latency, representative package/CDN hosts, small files, large files, warm cache, and interrupted/resumed downloads. Randomize or interleave repeated trials and report variability, not just the fastest run.

Record wall time, useful throughput, setup latency, request/connection counts, total bytes including probes/retries, CPU, peak memory, disk writes, and resumed bytes. Include publication and verification time in end-to-end results.

Release gates: no known compatibility regressions in admitted native cases; exact curl fallback for all unverified cases; no corruption in fault tests; bounded resources; verified crash recovery; and measured performance benefit on the intended workload. Choose numerical regression budgets from the baseline before evaluating the candidate.

There is no promised speedup. If parallelism adds load without useful improvement, keep pooled single-stream as the native path and leave chunking opt-in. Report how often real recipes remain on curl so performance claims reflect actual eligibility.
