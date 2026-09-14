# ADR 0077: Native capture pacing and caller completion cooldown

- **Status:** Accepted
- **Date:** 2026-09-14
- **Resolves gate:** none
- **Supersedes:** none

## Context

A caller's delay between OCR operations does not configure native capture work.
A watcher admission rate also does not wait after OCR and caller interpretation
finish. The requested workflow needs both controls without another scheduler or
mutable process-global configuration.

The facade already owns target-specific constructors and consuming
`NativeEngineRequest` options, but the architecture's blanket prohibition on
platform re-exports also excluded declarative OS configuration. That restriction
is narrower than the needed public contract; native handles and provider objects
must remain private, not configuration values.

## Decision

Permit target-gated `WindowsConfig` and `MacosConfig` re-exports only as immutable,
declarative native engine configuration. Keep neutral pacing requests/reports in
capture contracts, resolve defaults in the existing facade constructor inners,
and negotiate each session in its native provider. Runtime remains unaware of
concrete OS configuration.

Select one whole request in the order session > selected OS > common engine >
source default. Inheritance and explicit source-default reset differ; setters
replace whole selections/blocks. Required/preferred durations are positive, and
only selected values receive target representation checks before expensive work.
Existing constructor/default/const-value behavior is preserved.

Windows establishes `MinUpdateInterval` before callbacks/start using upward-rounded
signed 64-bit 100ns ticks. macOS establishes `minimumFrameInterval` before stream
creation/start with exact signed 64-bit nanoseconds and integer `CMTime` conversion.
Readback must not shorten the requested minimum. Only verified capability absence
permits an unapplied preference; real configuration and lifecycle errors still fail.
Session reports describe configuration, never measured FPS. Resize retains the
negotiated interval without a callback-side timer, software drop gate or additional
producer-storage retention.

Extend only the private macOS scalar handshake and matching version 22 layout
checks. Public C ABI 1.5, old-header prefixes and C++ default projections are
unchanged. Replay/ControlledCapture truthfully reject required native pacing and
report preferences unapplied without changing publication behavior.

Keep the completion loop in the Rust example: exact-frame OCR, bounded caller
interpretation, release unnecessary owners, full extra cooldown, then newest
same-stream observation after the last checked stamp. Use one absolute operation
authority and a local bounded 2ms polling wait. Preserve live-idle eligibility,
terminal-first acquisition, existing one-shot commit rules and independently
bounded cleanup. Add no public timer, watcher policy or automatic input.

## Alternatives

- **Reader throttling alone:** does not configure native callbacks/copies and
  cannot satisfy producer pacing.
- **A second builder or generic options interpreter:** duplicates the existing
  consuming request without adding capability.
- **Concrete defaults in runtime:** spreads platform selection into orchestration
  and separates direct-provider negotiation from facade behavior.
- **Separate interval and required flags:** can merge contradictory policies from
  different layers. Atomic selection preserves intent.
- **A public OCR runner/timer:** takes ownership of caller interpretation and adds
  an abstraction demonstrated by only one example.

## Consequences

Existing callers need no migration. Opt-in callers inspect the immutable session
report, keep input policy explicit and accept that longer intervals may miss
transient states or increase detection latency. Native conversion and private
bridge changes must remain coherent; the public foreign ABI has no pacing option.

The same dependency allowlist remains enforced. Private example support uses the
existing testkit dependency and its contract re-export pattern rather than a new
facade-to-capture development edge. Architecture, usage guidance, README and both
hosted target smoke steps are synchronized.

No performance budget, OS floor, permission behavior, historical evidence or
native support decision changes. A separately authorized five-case comparison
must bind workload, source/binaries, native inputs, hosts, intervals, duration,
cleanup, metrics and budgets before observation. Missing metrics are not zero.

## Verification

Observed local checks include 361 capture/replay/testkit tests (one intentional
watchdog-child ignore, with its parent test passing), 178 runtime tests, four facade
configuration tests, 20 completion-loop tests and the executable controlled smoke.
Eleven no-capture macOS tests exercise the production configuration/start/resize/
close path with call-local Objective-C doubles; two linked Rust/C agreement tests
check sizes/offsets. Windows cross-target Clippy passes but is not Windows runtime
proof. C/C++ ABI 1.5 ownership, frozen 1.0/1.2/1.3/1.4 headers and CMake consumers pass;
real-model foreign examples are compiled but deliberately not run.

Local OS is macOS 26.6.2 (25G83); the installed compiler selects SDK 27.0. This does
not qualify macOS 27 or replace revision-bound SDK 26.5 native evidence. Hosted CI
runs deterministic checks on both release targets. Native pacing and the
capture-off/source-default/cooldown-only/native-only/combined resource comparison
remain unexecuted. [The usage guide](../capture-pacing.md) records that boundary
and the exact caller invocation.

## Qualification apparatus correction: report schema 2

The verification paragraph above records the initial `06d845a` implementation
snapshot. Later version-one native attempts are retained separately, including
permission refusal, fixture startup failures and the first Windows observations;
none establishes a completed five-case comparison.

Windows campaign 02 reported one `sample_losses` event with an approximately
241.7ms process-sampler gap. Inspection showed that the field counted crossed
100ms polling deadlines **after aggregating a successful observation**. There
is no process-observation queue to lose that sample. CPU is cumulative, RSS
includes the OS high-water mark, and native-copy intervals have independent
coherence/overwrite checks. Calling scheduler delay a dropped observation
incorrectly invalidated otherwise available evidence.

New consumer report schema 2 calls the same counter `missed_poll_deadlines`.
Its value, sample count and maximum gap remain visible; delayed polling does not
claim ideal 100ms coverage or continuous instantaneous peaks. Sampler/consumer
capacity failures, missing required memory evidence, fixture sample loss,
native-copy invalidation and every pacing, ownership, latency and cleanup gate
remain enforced. Do not replace the removed mislabel with a fabricated zero.

This is a separately predeclared apparatus protocol for new attempts, not a
rescore or budget edit of old results. Schema-one authorities, raw reports,
failure classifications and binaries remain immutable. The schema-two analyzer
rejects schema-one evidence rather than applying the corrected interpretation
retroactively. Product API behavior, OS support and historical product budgets
are unchanged.

## Fixture boundary correction: protocol revision 3

Normal macOS launches showed the fixture become frontmost after promotion to
`Accessory`, despite non-key/non-main windows and an inactive AppKit state.
Debugger launches did not reproduce that transition. Keep the CLI's existing
`Prohibited` activation policy instead of changing it. A diagnostic run using
that policy reached native capture without changing foreground.

The captured owned yellow marker was BGRA `[84, 255, 255, 255]`, not the original
near-primary color. Decode blue/yellow using a minimum 96-channel dominance
margin with widened arithmetic. This preserves a separated code alphabet after
color management; all 128 bits and the nonce still must match, and erased or
ambiguous colors fail. The corrected native diagnostic completed five OCR
observations, resize/retention and terminal/interruption checks with clean exit.
It is diagnostic evidence, not a substitute for the bound release campaign.

Windows pool recreation drops its transition frame and may require another
eligible publication. The resize fixture presents the initial resize and three
bounded repaints, each at least 125ms after the previous render completes, then
acknowledges the final one. This spans
the required 100ms pacing interval while leaving stop processing responsive.
The consumer still requires the actual final pixels and newer epoch/geometry.
This does not claim single-repaint-then-indefinite-idle resize support.

Protocol revision 3 keeps report schema 2 and every existing numerical resource,
latency, deadline and cleanup gate. Earlier protocol files, runs and failures
remain unchanged; only fresh, fully bound attempts use the corrected fixtures.
