# Application performance and runtime safety audit

Date: 2026-11-14. Reviewed the current working tree, including pending daily-word cache changes. GPT-6.1 Sol XHigh advised; GPT-6 Luna Medium inspected search/runtime paths and measured an isolated release baseline. This audit changes documentation only.

## Findings in recommended implementation order

### 1. Urban parser recursion can exhaust the stack

**Crash hardening.** `src/urban.rs:16–20,52–57` normalizes a definition and recursively unwraps recognized register prefixes. Repeated `internet slang;` prefixes cause response-controlled recursion, retaining progressively shorter string allocations on every frame. Ordinary Urban lookups and daily parsing both reach it.

An isolated optimized harness using the unchanged inference functions reproduced stack overflow and exit 134 with 3,000 prefixes, approximately 45 KB of input, on 256 KiB and 512 KiB stacks. The same input succeeded on 1 MiB and 2 MiB stacks. This demonstrates a stack-overflow/SIGABRT path, **not a SIGSEGV or a reproduced crash of the installed app**. The local GPUI Linux dispatcher uses standard Rust worker threads without setting stack size; the running application's effective limit was not measured.

Replace recursion with bounded iteration and avoid repeatedly normalizing the remaining string. Validate repeated-prefix, Unicode, and existing POS fixtures with runtime/allocation limits. A response-size cap alone does not fix the algorithm.

### 2. curl decode errors leave unreaped children

**Confirmed resource cleanup bug.** `src/model.rs:267` returns on a read/UTF-8 error before `child.wait()`. An exact-source harness with a fake curl emitting byte `0xff` returned the decode error and left a child in `/proc` state `Z`. Repeated malformed responses can accumulate zombies while the app remains open.

Add guaranteed kill/wait cleanup on every exit, including cancellation, byte-limit rejection, and read errors. Read bounded bytes before decoding. Test these paths with fake subprocesses and assert reaping.

### 3. Incoming responses have no memory limit

**Memory robustness.** `src/model.rs:266–267` reads unrestricted stdout into a String. Daily (`src/daily.rs:45–49,64–66`), Urban (`src/urban.rs:319–347`), and Wiktionary parsers deserialize responses before reducing them to displayed entries. A six-second deadline limits duration, not bytes. Urban's five-result limit and the daily cache's 1 MiB read cap do not bound incoming responses.

Add a configurable shared byte limit enforced during reading, plus field/rendering limits where needed. Retain good daily caches on rejection. Test oversized streams, missing content length, invalid UTF-8, and large valid responses; always reap the child.

### 4. Obsolete lookup jobs continue running

**Responsiveness under repeated searches.** `src/ui.rs:413–464` detaches jobs per submitted query. Sequence checks reject stale results but do not cancel queued/running decompression, requests, or optional Python classification. Rapid submissions can saturate the fixed GPUI worker pool and delay the latest query or theme sync. This is an unbounded application job queue, not an assertion of unlimited thread creation.

Coalesce duplicate in-flight queries, bound queued work per source, and check cancellation before expensive operations. Cancel/reap active subprocesses where appropriate. Validate with slow mock sources, request/process counters, and latest-query latency limits. Similarity scanning already has cooperative cancellation and stopped promptly in the sampled test.

### 5. Search repeatedly decompresses and tokenizes dictionary data

**Measured CPU cost.** `src/model.rs:143–156,180–187` consumes an entire initial-letter JSON object on every offline lookup, including records after a match. This controls retained memory but repeats decoding. `src/similar.rs:178–205` scans the full corpus; `src/model.rs:189–252` constructs owned entries/definitions for each candidate, then scoring tokenizes candidate definitions on each search.

Compare smaller generated buckets or a compact build-time lookup index, with a small byte-bounded result cache for repeat queries. For similarity, profile a compact token index against a specialized scan that avoids unnecessary owned Entry construction. Measure startup and retained memory before selecting a design; avoid caching entire decoded letters without a budget.

Candidate tokens are already reused across query definitions within one scoring call. The UI waits for pending Urban results before its initial scan, so normal parallel completion does not itself cause two scans. The retained pool's O(pool size) duplicate/weakest scans are secondary candidates; profile before adding a heap/index.

Do not simply return early from `Pick::visit_map`: serde_json still expects a closing map delimiter after the visitor returns. Preserve duplicate suppression, score/lexical tie order, weighted randomness, and cancellation.

### 6. Modal rendering builds the page 38 times

**Rendering CPU and transient memory.** `src/ui.rs:1298–1406,2074–2104` builds 37 still-page samples plus the live page. Current step 2.2/radius 7.0 yields 36 nonzero offsets plus the origin; the previous audit's count of 33 was incorrect. Each sample reconstructs content and layout during animation/repaint.

Investigate a cached rendered surface or toolkit blur while preserving current weights, offsets, text, and selection behavior. Reusing element builders may still repeat layout/paint. Require isolated visual comparisons and frame timings for home/results, long entries, resizing, theme changes, transitions, and reduced motion. Existing paint tests do not establish visual equivalence.

### 7. Theme reads trigger their own watcher

**Confirmed idle-work bug.** `src/ui.rs:702–711` marks every notify event dirty. `src/theme.rs:229` opens the watched file during sync. An isolated harness using installed notify 7 confirmed that each of four reads emitted `Access(Open(Any))`. These events can sustain synchronization every 250 ms instead of the intended one-second fallback.

Ignore read/access events and unrelated paths; preserve modifications, atomic replacements, removals, and watcher recovery. Keep periodic checks for followed palette/symlink changes. Validate reads stay clean, writes/renames trigger refresh, and followed sources update. No entity-lifetime leak was found in the current polling loop.

### 8. Primary-selection capture can block startup

**Startup robustness.** `src/main.rs:60–74` runs synchronous `Command::output()` before the GUI starts. The 80-character check occurs after complete stdout capture. A stalled selection owner can hang startup; a huge selection is allocated before rejection.

Use configurable short deadlines, bounded stdout, and guaranteed child cleanup. Preserve provider fallback and the explicit query. Test slow/oversized/invalid/missing/successful providers with fake tools, without touching the active clipboard.

### 9. Optional pronunciation delays usable results

**Network latency.** `src/daily.rs:17–38` makes four serial requests: current featured HTML, its pronunciation, previous featured HTML, and its pronunciation. Their combined timeout budget approaches 24 seconds if required responses succeed slowly and optional requests time out. Failure of the first fetch instead stops after its own timeout. Archive selection is local processing, not a fifth request. English Wiktionary definitions also wait for optional pronunciation (`src/model.rs:297–309`).

Fetch required text first; defer or parallelize optional enrichment with bounded concurrency. Preserve the two-word cache contract and good cached data on partial failure. Validate delayed enrichment and fresh-cache startup separately. Persistent caching already removes this chain from fresh-cache opens.

### 10. Urban rendering and text-based IDs duplicate strings

**Transient allocations.** `src/ui.rs:1682–1725` clones Urban definitions/examples into a temporary Entry per render. `src/ui.rs:2045–2062` embeds full definition/example/credit text in element IDs while separately owning display text. Modals multiply the cost. Daily rendering already borrows fields; startup cache hydration still briefly clones records (`src/ui.rs:279–286`).

Use a borrowed Urban rendering adapter and shared text where supported. Shorten IDs with structural identity plus bounded content fingerprints or explicit content-version handling. SelectableText retains selection handles by ID: removing content identity without invalidation risks stale selections. Dependency frame state clears between frames; no unbounded ID interning or application heap leak was demonstrated.

### 11. Production cache startup lacks an integrated test

**Validation gap.** `src/ui.rs:264–265,334–337` bypasses hydration/refresh scheduling under `cfg!(test)`. Format and update helpers are tested, but actual Dictionary construction with fresh caches and zero requests is not. Inject cache storage, clock, and fetch backends to exercise production scheduling with isolated state. Cover fresh/stale/invalid caches, failure, independent sources, and reopening.

README's data section also still says Urban text is never stored, contradicting persisted daily selections. Clarify that daily selections are cached and ordinary lookup results are not persisted. This audit leaves that documentation fix and application changes for implementation.

## Release baseline

Standalone offline harness imported current model, similar, urban, and urban_pos modules. Release settings matched the app: thin LTO, one codegen unit, stripped, panic=abort. No GUI, network lookup, classification, or user caches were used. Ten consecutive samples per offline query, five recommendation samples; even-sample medians below average the two middle observations.

| Operation | Samples | Median | Maximum |
| --- | ---: | ---: | ---: |
| Webster `happy` | 10 | 14.868 ms | 15.517 ms |
| Webster `serendipity` | 10 | 41.502 ms | 42.444 ms |
| Webster `zebra` | 10 | 1.132 ms | 1.167 ms |
| Webster miss `zzzzqqqq` | 10 | 1.135 ms | 1.179 ms |
| Similar `happy`, limit 12/pool 96 | 5 | 700.107 ms | 723.532 ms |

Whole-harness VmHWM was 9,236 KiB after these operations. This is **not GUI app memory, an allocation count, or incremental similarity memory**. One cancellation requested at 10.135 ms joined 0.066 ms later. These host samples are not a before/after speedup, percentile estimate, or guaranteed latency.

Exact source, manifest, and lockfile remain in `/tmp/dictionary-search-baseline/` for this session:

```sh
CARGO_TARGET_DIR=/tmp/dictionary-search-baseline/target cargo build --offline --release --manifest-path /tmp/dictionary-search-baseline/Cargo.toml
DICTIONARY_SIMILAR_LIMIT=12 DICTIONARY_SIMILAR_POOL=96 /tmp/dictionary-search-baseline/target/release/dictionary-search-baseline
```

The harness prints the upper middle observation for ten-sample medians; this table corrects that from captured raw samples. Temporary harnesses may disappear after reboot. Other proof sources are `/tmp/dictionary-notify-audit.rs`, `/tmp/dictionary-curl-child-audit.rs`, and `/tmp/dictionary-urban-recursion-audit.rs`. Stack-overflow runs disabled core dumps and used separate processes.

## Validation and limits

- `cargo test --offline`: 77 passed, 0 failed, 4 ignored; includes isolated GPUI fixtures.
- `python3 -m unittest discover -s scripts -p 'test_urban_pos.py'`: 8 passed.
- `bash -n scripts/install.sh scripts/setup-urban-pos.sh`: passed.
- `cargo clippy --offline --all-targets -- -D warnings`: failed with 13 diagnostics, mainly collapsible conditions, plus needless borrow, character-pattern style, manual multiple check, and an eight-argument function. These are maintenance findings, not crash reproductions.
- No application-owned unsafe block was found in `src/`; this does not establish dependency/driver safety. Wiktionary section depth is limited by accepted heading levels. Inspected unwraps generally have local invariants or fixed embedded data behind them.
- No active window, clipboard, real session service, or user cache was manipulated. No live endpoint tests, full GUI heap profile, sanitizer/Valgrind run, or renderer/compositor stress test ran. No application RSS/leak trend or SIGSEGV was established.
- Application code and installed binary were not changed, rebuilt, or reinstalled. Existing daily-cache edits were preserved. Prior bounded ranking, borrowed daily rendering, and polling-liveness improvements remain present; no before/after speedup is claimed.

Implement parser bounds and child cleanup first, followed by request coalescing, watcher filtering, and measured search improvements. Keep backdrop changes as a separate visual-validation task; capture isolated GUI memory/frame baselines before broader caching or rendering changes.
