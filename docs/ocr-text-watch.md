# Rust OCR text-presence queries

`Session::start_ocr_text_watch` waits for a literal in one explicit region of a
maintained session. This is an additive Rust implementation. The controlled
public-API smoke, model-free normalization proof, and real CPU replay on both
release targets have run. Windows WGC, Apple ScreenCaptureKit and new
workload-budget qualification remain separate open acceptance gates. No new C/C++
surface is provided.

The current Windows 11 and Apple Silicon macOS 26.6.2 hosts are
[supported OS baselines](architecture.md#os-support-policy). These feature gates
and earlier-run failures do not classify those hosts as unsupported. macOS 27.0
verification belongs to a separate Change and is not inherited from macOS 26.

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
ONNX Runtime. Windows observes every module, including system DLLs; macOS
observes every non-system image, with shared-cache images bound to the OS build.
The approved and observed maps must agree except for the Windows presence-only
rule below. This binds OpenCV as well as ONNX. Windows records a bounded
current-process module snapshot; macOS collects a dedicated diagnostic pipe into
an exclusive private file bounded at1MiB. Ordinary stdout/stderr keep their
separate budget. Inherited OS file limits are unchanged, without imposing a new
limit on unrelated writes. Whole-process timing includes the collector; Rust
memory rows describe the native candidate, excluding the Python collector.
System shared-cache paths are removed before applying the256-entry Apple bound.
Neither an unchanged executable nor a version string replaces actual identity.
[ADR 0072](adr/0072-separate-bounded-loader-image-evidence.md) retains the earlier
output-limit failure and global-limit mechanism.
[ADR 0074](adr/0074-isolate-darwin-loader-image-budget.md) records its replacement
after an unrelated-write failure; neither decision promotes old failed runs.
Windows snapshots do not establish the history of transient unloaded images;
the required OpenCV and ORT dependencies remain loaded for this procedure.

Windows identity keys use the resolved extended-length spelling: `\\?\D:\...`
or `\\?\UNC\server\share\...`, matching the native consumer convention.
Literal trailing dots/spaces remain distinct names. Use the recorded identity
keys rather than constructing aliases; manifest validation accepts only that
canonical spelling. A path-key correction does not promote a failed binding.

Windows OS-managed `apphelp.dll` presence or absence alone does not reject an
otherwise successful execution. The sole exception is the canonical
`apphelp.dll` path within the actual OS system directory obtained read-only
through `GetSystemDirectoryW`, using the existing path canonicalization.
A hardcoded `C:\Windows`, an environment-derived directory or a basename match
cannot establish that identity. Failure to identify the path is a fail-closed
apparatus failure.

Exclude only that key from symmetric missing/extra-image equality, not from
observed image/hash evidence. Hashes of all common images must match, including
`apphelp.dll`; every declared file identity and hash fence still applies even
when the declared system `apphelp.dll` was not observed. Every other missing or
extra image remains a failure. Private observation metadata retains the full
`observed` map and report identity and adds `os_managed_presence_exclusions`:
the sole identified key on Windows and an empty list on Darwin. No DLL loading,
pinning, injection or OS/configuration change is part of this policy; Darwin
acceptance is unchanged.

Process success, semantics, physical cleanup, deadlines, output limits and
source/executable/runtime/model/fixture/input identity gates remain mandatory.

When the reviewed image map is not yet available, a separately authorized
binding-only run may execute the qualification-feature transition example **once**:

```sh
: "${OCR_BINDING_EVIDENCE:?new private binding evidence directory required}"
python tools/setup-native.py -- python tools/ocr-text-watch/bind_replay.py --mode real-cpu \
  --executable "$OCR_REPLAY_EXAMPLE" --corpus "$OCR_REPLAY_CORPUS" \
  --output "$OCR_BINDING_EVIDENCE" --execute-binding
```

It has the same90-second process,10-second cleanup and64-KiB output bounds.
The separate native-image report is bounded at1MiB; saturation remains nonpass.
It records exact source/inputs, complete output, required executable/runtime
membership and observed image hashes. Additional images are observed and hashed
after execution, not preapproved or proven unchanged throughout this binding run.
The resulting `observed-native-images.json` must be independently reviewed before
it becomes an approved input to the later three-process replay cohort. Binding
success is neither image approval nor replay/numeric/native qualification.
Failure retains the first attempt and authorizes no retry.

For the controlled executable's own image set, use a separate model-free binding:

```sh
: "${OCR_CONTROLLED_EXAMPLE:?exact controlled workload executable required}"
: "${OCR_CONTROLLED_BINDING_EVIDENCE:?new private binding directory required}"
python tools/setup-native.py -- python tools/ocr-text-watch/bind_replay.py --mode controlled \
  --executable "$OCR_CONTROLLED_EXAMPLE" --output "$OCR_CONTROLLED_BINDING_EVIDENCE" \
  --execute-binding
```

This runs one fixed `--semantic` workload process with300 seconds,15 seconds of
cleanup and1MiB ordinary output. It accepts no corpus or model initialization.
The canonical workload profile, harness and executable identities are fixed
before execution. Its observed image set needs independent approval; this
discovery run is not one of the three subsequent precursor samples. A static
import closure or another executable's observed set is not a substitute.

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
Structured per-invocation rows keep backend-input view traffic, caller-accessor
mapping traffic, logical cache/result extents and OS memory separate. They do
not claim unique allocations or a total opaque native-mapping ledger.
[ADR 0071](adr/0071-ocr-watch-observable-measurement-scopes.md) defines the scopes.

`tools/ocr-text-watch/workloads.py` requires a clean committed candidate, a
reviewed native image manifest under the presence policy above and a private host
record. It runs three controlled processes or, only in explicit
`real-cpu-cold-startup` mode after separate authority, five fresh runs of the real
transition example. Host declarations
are separated from observations; only the named host and release target are
verified by this runner. Unobserved CPU/memory facts remain unverified.
Real startup requires an example built with the nondefault
`ocr-text-watch-qualification` feature. It reuses the existing public constructor,
ONNX initialization hooks and native memory sampler to record twelve ordered
process-local stages through physical OCR zero and retained-owner release.
Observed detector/recognizer creation counts are not live-session counts.
The supervisor's whole-process cleanup remains separate from internal timestamps
and ORT process-global residency. Ambient `MADO_PILOT_ORT_PROFILE_DIR` is refused.

Report schema4 separates complete measurements from numerical acceptance. Every
controlled invocation now retains its exact ordered endpoint durations, including
the mapping/inference cancellation pair, from the existing producer output.
Missing/duplicate/malformed records stop later processes; no missing value becomes
zero. A numeric failure does not erase otherwise complete measurement facts.

`--enforce-budgets` requires accepted target-specific limits and their accepted
ADR, the exact approved host and executable, and unchanged profile/ADR/source/input
identities before every process and finally. An unaccepted profile always refuses.
Each process is compared independently; a first numeric failure stops the rest.
`passed` applies only to that complete target/mode cohort. Without the flag, the
runner collects measurements but cannot qualify a budget. Native capture and
whole-Change acceptance are never inferred from a single cohort result.

Admission under the Windows presence-only policy requires a new, separately
authorized three-process controlled cohort. Diagnostic rows are not precursor
samples, and a failed or interrupted cohort is not resumed under the new policy.
Every prior `NONPASS` and sealed artifact remains bound to its original run.

## Native and workload gates

The independent [Windows procedure](ocr-text-watch-windows.md) and
[Apple procedure](ocr-text-watch-macos.md) use new owned fixtures and acknowledged
text transitions, not frozen foreign controllers. Windows adds no permission
probe or elevation. Apple requires an already granted non-prompting Screen
Recording decision, never Accessibility. Permission refusal, unsupported-system
refusal and absence of an acknowledged matching frame are distinct non-passes.
Compilation in hosted CI is not evidence that either capture path ran.

The [Apple workload](benchmarks/ocr-text-watch-aarch64-apple-darwin.toml) and
[Windows workload](benchmarks/ocr-text-watch-x86_64-pc-windows-msvc.toml) now have
complete controlled3 and real-startup5 precursors. The
[G-013 budget decision](adr/0075-ocr-text-watch-workload-profiles.md) accepts
target-specific ceilings before the still-unexecuted final enforcement. All eight
workload blocks use process-local observations, not pooled samples or substituted
diagnostic rows. Earlier OCR/template budgets
and historical passes or failures remain revision-bound and do not qualify this
capability.
