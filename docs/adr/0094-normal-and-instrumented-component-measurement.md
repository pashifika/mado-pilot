# ADR 0094: Separate normal component timing from instrumented evidence

- **Status:** Accepted — apparatus contract only
- **Date:** 2026-10-23
- **Resolves gate:** _none_; target-specific observer eligibility and numerical profiles remain separate decisions
- **Supersedes:** _none_; ADR 0086's historical evidence and ADR 0087's release obligations retain their scope

## Context

Rust allocation/work accounting is useful for explaining a matching change, but
its cost cannot establish production-allocator timing. Likewise, a loaded
OpenCV endpoint list does not observe a library that loads and unloads between
snapshots. Discovery, semantic correctness and timing eligibility are different
claims.

OpenCV's `getBuildInformation` describes compiled support, whereas
`currentParallelFramework` reports the selected runtime parallel backend.
OpenCV's own [version utility](https://github.com/opencv/opencv/blob/4.14.0/apps/version/opencv_version.cpp)
forward-declares that exported diagnostic function and pairs it with
`getNumThreads`. The [implementation](https://github.com/opencv/opencv/blob/4.14.0/modules/core/src/parallel.cpp)
also shows why the GCD compatibility value is not a supported thread-setting
observation.

## Decision

1. Build normal and instrumented component executables separately. Normal uses
   the system allocator without matching/mapping accounting. Instrumented code
   retains allocation/work/ownership evidence. Both use the same first-party
   fixtures and public-result oracles. Component protocols record compiled mode
   and unavailable fields explicitly; feature-unified instrumentation rejects a
   normal build at compile time. Historical protocols retain their interpretation.
2. Observe the runtime backend and OpenCV parallel-region thread setting before
   and after component work. An optional, benchmark-only C++17 scalar boundary
   contains C++ exceptions and copies a bounded backend name into caller-owned
   storage. It neither sets thread limits nor exposes a new SDK API or C ABI.
   Missing, unsupported, invalid or changed observations make the executable and
   collector refuse success while preserving available JSON. This is the
   library-reported setting, not a count of workers actually executing. Roles
   must use equal effective backend/thread settings before launch and on retained
   evidence revalidation.
3. Treat runtime discovery as a prerequisite, not acceptance timing. The macOS
   component helper uses child-only `DYLD_PRINT_LIBRARIES`, the runner's actual
   root PID, raw loader records, Mach-O UUIDs and file identities/hashes. Shared
   cache classification requires observed cache membership, not a path prefix;
   observer OS/cache identity is retained. Unknown, truncated, foreign-process
   or unbound records refuse closure. Unload chronology and manual mappings are
   outside this method. Windows debugger-based discovery binds held image-file
   objects but remains preflight-only; it does not qualify normal timing.
4. Bind every selected representative path to the executable and declared
   dependencies. Retain preflight bundles and observation receipts with source,
   mode, host, environment and finite execution bounds. Compare each measured
   run's load records and pre/post file bindings with its frozen closure.
   Execution, cleanup, provenance and semantic outcomes remain independent;
   a nonzero exit does not erase verified post-run file bindings. File/OS
   immutability, loader coverage, observer cost and initialization/cache effects
   require an explicit source-bound eligibility review; successful discovery
   alone does not establish these assumptions or observational equivalence.
5. Require an explicitly accepted, externally hash-bound method review before
   calibration. Collect six independent baseline processes, then a separate
   balanced six-pair unchanged A/A control. Missing calibration prerequisites
   prevent control launch and cannot yield a passing control report. Accept a
   separate numerical profile before six-pair candidate comparison. Known
   failures remain failures when another observation is unavailable. No failed
   attempt is replaced, no candidate result widens its own threshold, and
   instrumented work reduction does not substitute for normal p95/CPU benefit.
6. Limit recorder and no-spawn restrictions to the named benchmark campaign.
   Applications choose their targets, windows and consumer-created children.
   The SDK owns its documented parameter contracts and its own resources and
   workers; applications need not adopt these recorders. Component evidence is
   not consumer-startup, UI, capture, input or arbitrary-application evidence.
7. Use protocol 3 for the successor Session corpus. Keep logical input, request,
   ordering and timing/owner contracts separate from each role's helper-source
   hashes and implemented physical accounting. Validate all declared turnover
   strata as well as their overlapping cycle, without adding them. Query close
   and retained-terminal verification are outside its primary wait interval;
   lifecycle contracts include their entire owner script. Normal unavailable
   fields remain unavailable, and two independent caches do not widen a
   per-backend bound or the final-owner residual allowance. The collector retains
   protocol-1/2 readers; current binaries emit only protocol 3.
8. Admit two fixed, separately source-bound physical contracts:
   `prepared-cache-row-winner-v1` and `uncached-bgra-repeated-scan-v1`. A
   measurement-only historical port preserves old BGRA mapping, uncached BGR
   preparation and repeated selection scans while sharing the logical corpus
   and public-result oracles. Zero historical cache gauges do not describe its
   temporary native matrices. Logical mapped-view, copied and converted bytes
   remain separate validated quantities; common-manifest comparison removes
   only admitted physical-role differences.
9. Bind an exported measurement port to its own actual Git commit/tree, not
   the collector checkout's `HEAD`. Paired `--source-root`/`--source-revision`
   options verify the complete regular-file export before and after attestation
   through real recursive-tree/blob identities and bounded file hashes.
   Preserve the original historical source and artifacts. Refuse unexpected
   files, unsafe links, changed bytes or helper-source mismatches. Windows
   regular-file verification retains declared Git modes without inventing
   POSIX executable-bit observations.

## Alternatives

- Use instrumented timing as normal timing: combines the mechanism observation
  with the performance claim it is meant to explain.
- Label `getBuildInformation` as the active backend: confuses build capability
  with runtime selection.
- Promote endpoint snapshots or debugger discovery to normal qualification:
  leaves transient-image coverage or debugger-induced runtime changes unresolved.
- Apply campaign child restrictions to SDK consumers: adds an unrelated product
  constraint without knowing an application's intended scope.

## Consequences

The OpenCV backend gains an optional `cc` build edge for benchmark diagnostics;
production defaults, matching algorithms and public integration contracts do
not change. Controlled native versions must expose the diagnostic symbol: no
build-label fallback is used. Native C++ exceptions, unavailable observations,
file/loader drift, finite budgets, failed cleanup and retained-cohort admission
have explicit verification paths.

This ADR accepts no numerical budget, Windows normal-observation method,
consumer comparison role or release performance claim. Those require their own
prospective decisions and target-local evidence under
[ADR 0087](0087-v0-5-0-release-performance-qualification.md).
