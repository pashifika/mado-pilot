# OCR watch measurement method

Keep controlled scheduler/resource observations, real CPU initialization and
native capture verification separate. A complete measurement is not automatically
numerical qualification. Use the [performance guide](../../performance.md) for
ordinary tuning, empirical-target review and mandatory safety boundaries.

## Development observation

`tools/ocr-text-watch/workloads.py` can run without a numerical profile. It uses
the existing finite workload settings and records its own current source, host,
executable, native-image and input identities. It needs no accepted ADR,
calibration/A-A/profile approval, prior numerical-control pass or CPU-idle
threshold. It still requires a clean committed source, explicit candidate
executable, private host record, independently reviewed native-image manifest,
new private output directory and explicit `--execute` for the selected cohort.
These identity/privacy/process requirements do not supply numerical acceptance.
Windows private evidence directories require Python 3.13+.

For the controlled, model-free workload cohort:

```sh
python3 tools/ocr-text-watch/workloads.py \
  --mode controlled --executable "$CONTROLLED_BENCH" \
  --host-record "$HOST_RECORD" --native-images "$NATIVE_IMAGES" \
  --output "$NEW_PRIVATE_EVIDENCE_DIRECTORY" --execute
```

This runs three processes. Each runs `steady-nonmatch`, `positive-consecutive`,
`slow-backend-saturation`, `mixed-two-session`, `retained-results` and
`cancellation-close`, with two warmups and twenty retained samples per workload,
plus one controlled `cold-startup` sample. The generated BGRA source is
`960×540`, `2073600` bytes, full-frame ROI; query minimum interval is 50 ms and
capture period 16 ms. Scripted OCR/explicit handshakes establish scheduling
semantics, not real model latency or 16-ms native-capture throughput.

For a separately selected real CPU startup observation:

```sh
python3 tools/ocr-text-watch/workloads.py \
  --mode real-cpu-cold-startup --executable "$QUALIFICATION_ENABLED_EXAMPLE" \
  --corpus "$FIXED_REPLAY_ROOT" --host-record "$HOST_RECORD" \
  --native-images "$NATIVE_IMAGES" \
  --output "$NEW_PRIVATE_EVIDENCE_DIRECTORY" --execute
```

This runs five fresh transition-example processes, zero warmups and one sample
each. The executable needs the nondefault `ocr-text-watch-qualification` feature
and fixed prepared corpus from the
[real-replay procedure](../../ocr-text-watch.md#fixed-real-replay-procedure).
It reuses the public CPU bounded-v2 constructor and qualification-only ONNX
initialization hooks/native memory observation; it does not add normal-runtime
sampling. The example's query interval/confirmation overrides are 1 ms/one.
Ambient `MADO_PILOT_ORT_PROFILE_DIR` is refused. Fresh processes do not imply
cache-cold storage: identity hashing can warm files; no cache flush, priming or
retry is part of this method.

Both modes retain 300-second process deadlines, 15-second cleanup, 1-MiB bounded
combined output and 1-MiB native-image reports. Native children remain owned by
the existing runner. The manifest's actual loaded-image/hash policy remains
strict, including only the documented Windows canonical-system `apphelp.dll`
presence exception; common hashes and declared disk identities remain bound.
Only host identity and release target are verified from the private host record;
CPU/OS/memory declarations remain distinguished from actual observations.
Source, documents, input/artifact/native identities are checked before each
process and finally. Semantic/identity drift, timeout/output limit, malformed or
missing required records and failed cleanup are non-success and stop later work.
Nullable OS RSS/peak/footprint/private observations can remain null in ordinary
workload collection; retain available values and reasons without a numeric
claim. This tolerance does not admit invalid zero, malformed wire data, wrong
order/count or missing logical fields, and does not change strict binding/replay
defaults.

A numerical failure does not erase valid measurement facts or prohibit a new
separately identified safe diagnostic experiment. Preserve failed cohorts,
unrun slots and any unresolved identity/ownership fence; a new directory is not
a cleanup bypass. Do not retry a fixed qualification cohort to obtain a pass.

### Reports and optional empirical comparisons

Current workload reports use `schema_version = 5`:

- `execution_passed` validates a nonempty attempted prefix: admission and final
  identity checks pass, every attempted row has valid semantics, required
  measurements, dependencies and cleanup, and no non-budget failure occurs.
  It can be true after numerical enforcement stops an otherwise valid prefix;
  it does not assert full cohort completion.
- `stopped_by_enforcement` identifies early stop for `budget-exceeded` or
  `budget-unavailable`. `observation_completed` and full-cohort fields remain
  false until every planned row completes.
- `measurement_gaps` is independent of profile selection. Each process row
  retains applicable-target OS-memory gaps with `target`, `field`, controlled
  workload/phase/iteration or real stage, zero-based parsed `record_index` and
  `reason`; the top-level list additionally identifies `process`. Lists are
  bounded to 399 controlled or 36 real entries per process, including malformed
  extra-stage input. Other-platform not-applicable fields are excluded. Raw
  nulls remain unchanged, and safe completed observations can succeed with
  these optional gaps.
- `empirical_targets.requested` means `--profile` was supplied, including a
  malformed or historical execution input. `.passed` is false if any compared
  row misses, including alongside unavailable observations; true only after a
  complete passing comparison with no unavailable measures; otherwise null.
  `.misses` retains process and comparison details. `.unavailable` lists
  process/workload/measure/reason; no null becomes zero.
- `numeric_budgets` is `not-enforced` when enforcement was not requested.
  `passed` and each process's `budget_passed` remain qualification-only fields,
  not synonyms for safe execution or completed observation. Explicit enforcement
  still returns non-success on a miss or unavailable required numerical measure
  even when `execution_passed` is true.

To inspect empirical targets for the exact profile-bound host/executable, add
`--profile docs/benchmarks/ocr-text-watch-v3-<target>.toml` to a command above.
An empirical miss stays visible without alone causing non-success or stopping
later otherwise safe observation processes. Supplying a profile is an explicit
scoped comparison: mismatched host/executable refuses rather than silently
applying another candidate's targets. For a new candidate/host omit the profile
and retain that experiment's own identity/limitations.

## Measurement boundaries

| Quantity | Scope |
| --- | --- |
| Backend-input mapped bytes | Actual CPU-view lengths arriving at the controlled backend, including a mapped request rejected as Busy; admitted-only counts remain distinct |
| Retained-accessor mapped bytes | Actual mapping descriptor lengths returned to caller-retained frame access |
| Cache high-water | Runtime logical cache extent, not unique allocations |
| Result/source/text/index and caller-frame extents | Logical retained ownership; aliases do not become additional physical storage |
| Native process memory | RSS/peak plus Apple footprint or Windows private bytes at named checkpoints, including retirement; sampled maxima and OS process high-water are not interchangeable |

No view sum is total physical mapping or native allocation. Rust allocator
accounting does not cover all ONNX/platform storage. A sampled high-water or
finite endpoint difference proves neither all-native leak freedom nor zero
process-global residency.

Real startup retains twelve ordered internal clock/memory stages and a summary:
runtime/provider and exactly one detector/recognizer construction, engine/session
readiness, query terminal, logical close, physical OCR zero, parent release and
retained-owner release use one process-local monotonic origin. Exactly one ready
hook per session proves no extra construction in the observed interval, not
session destruction or a live-session count afterwards. Logical close may return
while a physical lease remains; final physical OCR must be zero. ORT
process-global residency can persist until process exit.

The supervisor independently observes launch through whole-process cleanup.
Apple image collection is included in supervisor elapsed time, excluded from
Rust intervals/native-memory observations. Real intervals keep their actual
boundaries: `query_terminal_to_logical_close_returned_ms` includes intervening
result checks and is not isolated API-close latency. Never combine different
clock domains or rename the supervisor duration as OCR inference.

Structured records require exact order/count/types and all required endpoints.
A process failure retains precedence over missing telemetry caused by that
failure. Missing/duplicate/out-of-order records and absent qualification-only
instrumentation cannot become a complete measurement.
`required_measurements_complete` covers mandatory semantic/stage/logical records;
optional OS-memory gaps are reported separately as `measurement_gaps`. Workload
observations retain explicitly nullable OS-memory fields as null, including in
profile-free raw records; their absence is neither a malformed record nor a
passing numerical measurement. All available values, misses and unavailable
reasons remain visible.

## Optional budget enforcement

Add `--enforce-budgets` **and** a matching current `--profile` only when explicitly
requesting the fixed target/mode numerical check:

- [Apple successor profile](../../benchmarks/ocr-text-watch-v3-aarch64-apple-darwin.toml)
- [Windows successor profile](../../benchmarks/ocr-text-watch-v3-x86_64-pc-windows-msvc.toml)

These are `format_version = 3`. Explicit workload/host/artifact and numeric
settings carry the runnable contract. `bindings.host_id`,
`bindings.controlled_executable_sha256` and
`bindings.real_executable_sha256` scope the profile. `review.tuning_record`
links [rationale](../tuning/2026-10-26-ocr-watch-profile-authority.md) but is not
opened, parsed for status or used as an approval oracle. No current command
opens a retired ADR or planning archive to authorize execution.

Enforcement requires the exact scoped host/executable and source-bound profile
bytes, strict native-image/source/input bindings and complete required semantic,
measurement and cleanup checks. Compare each process independently; the first
numerical miss or unavailable required numerical measure returns non-success and
stops later processes. Available misses remain failures even alongside unavailable
values. A safely executed prefix can still have `execution_passed=true`; that
does not pass numerical enforcement. Final identity failure remains a failure
even after favorable comparisons. Only a complete
separately selected target/mode cohort can pass; neither precursor completeness
nor a single successful process supplies that verdict. No native capture,
whole-Change, platform-support or release claim follows.

The successor profiles preserve numerical values and original precursor
provenance; retaining those settings does not qualify a new source/host/binary.
Their empirical derivation remains:

- Timing ceilings: twice the worst corresponding process-local precursor
  statistic, rounded upward to `0.001ms`, minimum `0.001ms`. Controlled p50/p95
  use nearest ranks 10/19 from exactly twenty samples; independent maxima use
  all twenty. Controlled startup has one sample.
- Maximum ceilings: separate `1ms` minimum for short control-path maxima and
  real stage-interval maxima, not p50/p95. These are not scheduling hard-real-time
  guarantees; typical and whole-workload limits stay independent.
- Cancellation: take the larger mapping/inference endpoint **within each
  invocation**, then form that process's twenty-value p95/maximum; forty
  endpoints are not forty independent samples.
- Native memory: `1.25` times target-specific maxima rounded upward to `1MiB`,
  including warmups and every real stage. RSS, lifetime peak, private bytes and
  footprint remain separate; absent platform counterparts are not zero-filled.
- Fixed-fixture input/cache/source/text/index/frame/accessor extents use their
  observed maxima and ownership oracle. They are not arbitrary-caller allocation
  bounds. Cadence-dependent backend-view traffic stays evidence, not an invented
  aggregate performance ceiling.
- Physical OCR high-water is at most one. Logical close may still have one lease;
  its ceiling remains one even when a precursor observed zero. Final physical
  OCR is zero. Exact terminal/source authority, no late confirmation/diagnostic
  escape, finite pending/physical shape, original queue age and disposition
  accounting remain semantic requirements. Deliberate supersession/expiry need
  not be zero; no missing stale/drop/growth counter is invented.

An empirical target can be changed prospectively through ordinary review and a
short tuning record. That cannot relax runtime bounds, reinterpret a past
failure, discard an endpoint or authorize retrying a failed qualification.

Original format-2 profiles are read-only historical inputs. Current execution
refuses them with a migration diagnostic. Inspect without launching or loading
historical prose:

```sh
python3 tools/ocr-text-watch/workloads.py --inspect-profile \
  docs/benchmarks/ocr-text-watch-aarch64-apple-darwin.toml
```

Inspection returns the original parsed document and byte identity, not a
current acceptance verdict. The legacy Rust TOML registry/format-1 hard-budget
constants are not a second OCR enforcement authority.

## Native verification

Use the [Windows WGC procedure](../../ocr-text-watch-windows.md) or
[Apple ScreenCaptureKit procedure](../../ocr-text-watch-macos.md) for owned native
fixtures, acknowledged transitions, exact target generations and finite teardown.
They do not run through the controlled/real-startup budget result. Apple needs
already-granted non-prompting Screen Recording, never Accessibility for this
capture-only procedure; Windows adds no permission probe or elevation.
Permission refusal, unsupported system and no acknowledged frame are distinct
non-passes. Hosted compilation is not native execution evidence. Keep
[OS support](../../architecture.md#os-support-policy), native semantics and
numerical qualification separate.

## Historical outcomes

These records are explicitly historical, not current admission authority:

- [Precursor/ceiling record](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/adr/0075-ocr-text-watch-workload-profiles.md):
  Apple M1 Pro/macOS 26.6.2 and Core i7-12700KF/Windows 11/base build 26200 each
  supplied controlled3 and real-startup5 precursors. Windows controlled records
  25H2/build 26200.9445; the earlier real record omits UBR. Do not attribute a
  later registry value backwards. Earlier Windows controlled, Apple `SIGXFSZ`
  and loader-diagnostic attempts are not substituted samples.
- [Final numerical outcomes](https://github.com/pashifika/mado-pilot/blob/646369fad6c922ac7b064367817337a18e572d8c/docs/ocr-text-watch.md#native-qualification-and-numerical-outcomes):
  Apple controlled3/real5 and Windows controlled3 passed in their recorded scope.
  Windows real process 1 measured `1.4887 ms` against the `1.0 ms`
  logical-close-to-physical-zero ceiling; that cohort remains `NONPASS` and
  processes 2–5 remain unexecuted. It is not a current-source measurement or
  watcher-admission rule.

The tracked profiles condense source-bound observations, not complete raw-output
or allocation ledgers. Native verification has its own exact source/host scope
in the topic guides. Profile migration supplies no new measurement, retroactive
pass or universal model performance requirement.
