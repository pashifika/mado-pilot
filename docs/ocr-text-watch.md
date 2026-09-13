# Rust OCR text-presence queries

`Session::start_ocr_text_watch` waits for a literal in one explicit region of a
maintained session. This is an additive Rust implementation. The controlled
public-API smoke and model-free normalization proof have run; real CPU replay,
Windows WGC, Apple ScreenCaptureKit and new workload-budget qualification remain
separate, unexecuted acceptance gates. No new C/C++ surface is provided.

## Selection and request

Initialize the engine once through `replay_engine_with_ocr_profile`,
`windows_engine_with_ocr_profile` or `macos_engine_with_ocr_profile`, using
`OcrProfileConfig` with `OcrProfile::BoundedDetector`. Supply the existing
controlled model root and ONNX Runtime 1.29.0/API 17 path. No dependency is
acquired, bundled or searched for implicitly.

Start binds the complete initialized backend/model/profile/provider descriptor,
not a constructor name or just a profile string. The active provider must be
CPU and the profile must be
`phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2`. Missing OCR is unavailable;
native G-004, an accelerator or incomplete identity is unsupported. Existing
one-shot constructors and defaults do not change or fall back for a query.

`OcrTextWatchRequest::new` explicitly names:

- A nonempty coordinate-qualified `Rect`, `ClipPolicy`, and result coordinate space.
- A borrowed literal, normalized during construction, and finite confidence in `0.0..=1.0`.
- `OcrTextAnalysisRate::from_minimum_interval` with a positive interval.
- `OcrTextStability::immediate()` or `OcrTextStability::consecutive` with a positive count.
- The closed `ChangeDetectionPolicy` and a query-lifetime `OperationContext`.

No deadline means a deliberately deadline-free query. Start performs neither
inference nor a blocking capture to manufacture an initial transform. Intrinsic
region, fixed capture-origin, capability and option errors fail before
publication. The open-time extent is not current geometry after resize; mutable
upper bounds and transforms resolve against the first exact maintained frame.
A native source may supply no initial pixel-bearing frame for an unchanged window.

## Literal and result semantics

The literal uses NFC followed by Unicode edge-whitespace trimming. Only the
normalized `1..=4096` UTF-8 bytes are retained. Arbitrarily long edge whitespace
is accepted without allocating proportional text; processing is linear in input
and checks cancellation/deadline while scanning.

The Unicode 17.0.0 derivation examines all 1,112,064 scalars, including all
11,172 Hangul syllables. Its maximum canonical decomposition is four scalars;
preflighting at most 16,384 decomposed scalars cannot exclude a valid literal.
It also proves borrowed edge trim commutes with NFC for the pinned tables.
The fixed bounds describe requested working allocation, not allocator metadata,
RSS or recoverable out-of-memory behavior of pinned NFC internals and `Arc`.

After complete OCR output validation, presence means a case-sensitive substring
inside one normalized region at or above the requested confidence. Threshold
equality is accepted; regions are never concatenated and no case/width folding,
regex, fuzzy comparison or whitespace collapsing is added. Empty output or a
valid nonmatch resets confirmation and remains pending. Malformed output fails
the whole analysis, even after an earlier candidate would have matched.

A success retains the exact target, frame/stamp/transform, effective ROI, output
coordinates, immutable `OcrResult`, complete initialized descriptors, normalized
predicate, confirmation facts and checked `u16` satisfying-region indexes. Text
is not copied into another result array. Explicit accessors expose it;
`Debug`, errors and diagnostics do not. Results, frame clones and mappings remain
readable after query/session/engine teardown and never substitute a newer frame.

## Progress, authority and finite scheduling

`poll` is nonblocking. `wait` takes an independent caller-wait context: its
interruption ends only that wait, checked before a terminal observation.
`cancel` is idempotent; dropping the sole query owner cancels pending work.
A committed terminal is immutable. Already latched source/session authority
precedes scheduler shutdown; for an otherwise open query, cancellation precedes
deadline, and both precede a competing overload, backend failure or success.

Only distinct accepted positive analyses confirm. Re-observation, elapsed time,
rate deferral and a change skip never confirm. Epoch or geometry revision changes
invalidate incompatible generations and reset confirmation. A newer compatible
pending frame alone does not invalidate an in-flight result. Normal source end
drains acquired final work, then closes an unsatisfied query; explicit close,
target loss or interruption can preempt the drain.

Both query classes share the existing limits: 256 live engine queries, 64 per
session, 16 sessions holding work reservations, two analysis workers, one latest
pending frame per query, a 64 MiB/256-entry mapping cache and 30-second eligible
queue residence. OCR occupies at most one physical lease from claim through
completion. It never creates another backend pair or retries one-shot `Busy`.
Ready classes and OCR sessions/queries rotate; a continuously eligible OCR query
receives a claim within `S * Q` successful OCR turns for `S` ready sessions and at
most `Q` ready queries per session, conditional on actual work return.

A mapping admission barrier prevents a template worker from entering a native
conversion held by OCR. Existing template mapping reservations drain first;
OCR inference releases that barrier so the second worker can run templates.
A held conversion leaves later work pending and subject to queue expiry, not
blocked inside native mapping. Source acquisition continues considering newer
frames. Replacement preserves eligible age; intentional rate deferral does not
consume eligible residence. OCR analyses are not coalesced; template coalescing
and template-only scheduling remain unchanged.

OCR reuses one BGRA mapping for exact visible-row comparison and inference.
[ADR 0070](adr/0070-ocr-watch-exact-bgra-change-evidence.md) proves the equality
preserving channel permutation; row padding is ignored and confirmation forces
new analysis even for unchanged pixels. A full source envelope and returned
mapping are checked against the 256 MiB OCR-watch safety limit.

`Engine::ocr_text_scheduler` reports fixed policy; `ocr_text_observation` and
query progress distinguish logical work from the physical OCR lease. Retention
counters sum live result-owned source/text/index extents. They are not
unique native allocation/RSS counts and do not include arbitrary separately
retained frame clones. Native retention must be measured independently.

## Logical close is not physical quiescence

Cancellation and logical close seal OCR admission, cancel generation authority
and wake waits without joining an unreturned OCR call. After dispatch is sealed,
only the worker still owning the physical OCR lease may outlive final Rust owner
release. Template and acquisition joins keep their existing behavior; the
mapping barrier prevents an indirect template join behind that OCR conversion.
Late completion cannot revive a query or emit success after shutdown.

The detached call still owns its frame, mapping, recognizer and physical lease.
Do not unload executing code or controlled native dependencies while it remains
outstanding. No forced thread termination, replacement worker or recovery retry
is provided. A permanently blocked call can retain one bounded execution until
process exit. Logical success or close alone does not prove native allocations
were freed; native evidence must establish physical return and cleanup.

## Fixed real-replay procedure

The [complete example](../crates/mado-pilot/examples/ocr-text-watch.rs) constructs
one real CPU engine, starts/polls a query, demonstrates an independently cancelled
wait, reads the terminal and retains exact pixels after all parents close.
It has no fake-backend fallback. The three fixed scenarios are blank-to-positive
(sequence 1 in epoch/geometry 0), negative final-source drain, and fixed 2x
target-logical projection. Replay identities begin at zero.

Generate fresh replay bytes using an explicitly prepared Python environment with
Pillow 12.3.0. The script verifies the existing repository-owned HUD PNG; it
performs no inference or capture and refuses to overwrite an output directory:

```sh
: "${OCR_REPLAY_CORPUS:?choose a new corpus directory}"
python tools/ocr-text-watch/replay_corpus.py --output "$OCR_REPLAY_CORPUS"
```

The HUD content is Apache-2.0 repository artwork with OFL-1.1 Noto Sans JP rendered
text; no font bytes are bundled. The input, normalized region order, satisfying
index and geometry oracle are fixed before execution, not learned from output.
The corpus conversion uses Pillow's MIT-CMU-licensed developer tooling only.

Build with a new dedicated `CARGO_TARGET_DIR` and the ordinary native setup
wrapper. After fresh real-model execution authority, set
`MADO_PILOT_G004_MODEL_ROOT` and `MADO_PILOT_ONNX_RUNTIME`, then run:

```sh
: "${OCR_REPLAY_EXAMPLE:?exact built example required}"
: "${OCR_REPLAY_CORPUS:?fixed generated corpus required}"
: "${OCR_NATIVE_IMAGES:?prospectively approved native image manifest required}"
: "${OCR_REPLAY_EVIDENCE:?new private evidence directory required}"
python tools/setup-native.py -- python tools/ocr-text-watch/run_replay.py \
  --executable "$OCR_REPLAY_EXAMPLE" --corpus "$OCR_REPLAY_CORPUS" \
  --native-images "$OCR_NATIVE_IMAGES" --output "$OCR_REPLAY_EVIDENCE" --execute
```

The procedure requires a committed clean candidate, pins the executable,
controlled dependencies and fixed corpus, and runs exactly three fresh processes
with no warmups/retry. Each gets 90 seconds plus 10 seconds of cleanup and 64 KiB
of combined output. It retains the first failure, leaves later rows unrun and
checks identities again. Private records include paths and complete process
output and are not automatically publishable. Process return is not a new
workload-budget or native-capture qualification.

The native image manifest is a reviewed map of canonical absolute paths to
SHA-256 values, with at most 256 entries. It includes the executable and real
ONNX Runtime. On Windows it includes every observed module, including system
DLLs; on macOS it includes every non-system image, with shared-cache images bound
to the OS build. This binds OpenCV as well as ONNX. Windows records a bounded
current-process module snapshot; macOS uses captured dyld loader output. Neither
an unchanged executable nor a version string replaces actual dependency identity.
Windows snapshots do not establish the history of transient unloaded images;
the required OpenCV and ORT dependencies remain loaded for this procedure.

Metadata reads are limited to 1 MiB, identity files to 1 GiB, and non-regular or
changing files are refused. Metadata is parsed and hashed from the same bytes.
Injected-library environment variables and ambient Git repository selectors are
refused. Python 3.13+ creates private evidence directories; reports are exclusive
and never overwrite an earlier attempt. A standalone Windows dependency-report
environment setting also requires a caller-selected private parent directory.

## Model-free workload apparatus

`ocr-text-watch-query` runs six controlled workloads with two warmups and twenty
samples each, plus one controlled startup sample. It uses generated solid BGRA
frames and scripted OCR, not the HUD model. Query clocks advance under explicit
handshakes; driver wall time is not real OCR latency or 16 ms capture throughput.
Every warmup/sample must satisfy its semantic oracle.

```sh
: "${CARGO_TARGET_DIR:?set a new semantic-check target root}"
python tools/setup-native.py -- cargo test --locked --package mado-pilot \
  --bench ocr-text-watch-query -- --test
```

The process watchdog is 300 seconds. Held-work tests distinguish original
pending age from the last replacement, leave both classes ready when checking
the next template turn, and account for missing resource reads after teardown.
Reported backend-input mapping bytes, retained-accessor mappings and cache
occupancy are separate from an unmeasured total physical mapping ledger.
Logical retained extents are not native allocation/RSS totals.

`tools/ocr-text-watch/workloads.py` requires a clean committed candidate, an exact
native image manifest and a private host record. It runs three controlled
processes or, only in explicit `real-cpu-cold-startup` mode after separate
authority, five fresh runs of the real transition example. Host declarations
are separated from observations; only the named host and release target are
verified by this runner. Unobserved CPU/memory facts remain unverified.
Real startup measures launch-to-owned-cleanup wall time, not internal engine
timestamps, native pair counts or real-example RSS. Missing measurements and
unaccepted numeric ceilings keep qualification false. `--enforce-budgets` refuses
instead of inventing a pass. First failures and successful prefixes are retained;
later processes are not run after a failure.

## Native and workload gates

The independent [Windows procedure](ocr-text-watch-windows.md) and
[Apple procedure](ocr-text-watch-macos.md) use new owned fixtures and acknowledged
text transitions, not frozen foreign controllers. Windows adds no permission
probe or elevation. Apple requires an already granted non-prompting Screen
Recording decision, never Accessibility. Permission refusal, unsupported-system
refusal and absence of an acknowledged matching frame are distinct non-passes.
Compilation in hosted CI is not evidence that either capture path ran.

The [Apple prospective workload](benchmarks/ocr-text-watch-aarch64-apple-darwin.toml)
and [Windows prospective workload](benchmarks/ocr-text-watch-x86_64-pc-windows-msvc.toml)
are unmeasured and non-normative. Exact host/harness bindings, separately approved
precursors, justified numeric ceilings and their accepting G-013 ADR are still
required before final enforcement. Earlier OCR/template budgets and historical
passes or failures are unchanged and do not qualify this capability.
