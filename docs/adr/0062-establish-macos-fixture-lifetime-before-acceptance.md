# ADR 0062: Establish macOS fixture lifetime before acceptance

- **Status:** Accepted
- **Date:** 2026-09-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The current-source Apple native template-watch process on revision `030398e`
ended terminal red during `retained_result_mapping`. Its fixture acknowledged
Stop, but bounded finalization failed and the accepted p95 and maximum latency
limits were exceeded. ADR 0060 retained that result without assigning a cause.

A cleanup-only disposable localization then observed Stop accepted immediately,
the authenticated process lost, and the retained workspace launch still
`Unknown` with almost the complete existing finish deadline remaining. The final
state remained authenticated lost plus launched `Unknown`, so conservative exit
handling consumed approximately ten seconds without proving that exact process
lifetime gone. Fixed-ordinal correlation subsequently placed the same cleanup
failure on every exact maximum sample. Conservatively removing only the
integer-millisecond cleanup component plus a one-millisecond rounding allowance
left p95 `3017.158708 ms` and maximum `3029.324333 ms`, below the unchanged
accepted limits `7221.614 ms` and `7343.78 ms`.

The reviewed cleanup result, correlation result, and correlation review are
retained by Rasen Change
`macos-native-template-watch-retained-result-cleanup-and-latency-repair` with
SHA-256 identities `d73329d2421aaba89da70bb9ddfd976ba1932d887baf3818a99dda97593c8d5a`,
`7bf81ce75c10c1c13ed956b5f22d55f8b5dcb3ccfe6c765ec1fb80dcee406c74`, and
`6514b893a1ec9025eb13273d3a5945b3eb8a4082a1f5de2d32b7f5f203e3f4f1`.
Independent review found no Blocker, Major, or Minor issue. These disposable
results select a repair; they are not qualification evidence and do not change
ADR 0060's `WITHHELD` support decision.

## Decision

After the private Unix peer authenticates and before either macOS fixture startup
path returns a controller, the retained workspace launch must be observed as
`Live` under the existing absolute launch deadline. `Unknown` waits on the same
launch attempt. `Lost`, observation failure, or deadline exhaustion rejects
startup and transfers the rejected owner to the existing exact containment path.
`start_once` never relaunches; the retry-capable development path applies the same
post-authentication acceptance fence.

Private fixture finalization returns one immutable typed result. It keeps Stop
acknowledgement, authenticated and launched lifetime observations, bounded exact
containment, reader/output drain, executable identity, and deferred-cleanup need
as separate facts. Every direct controller caller consumes that result before it
accepts a sample or process. `Drop` remains idempotent best-effort containment and
cannot convert an already observed failure into success. Failure diagnostics use
only fixed state tokens, booleans, counts, and durations; they contain no run,
process, application, display, path, or payload identity.

The accepted retained-result timer continues to include declared fixture
finalization and fresh-producer progress. No launch, operation, or cleanup
deadline; delayed-reaper limit; native exact-lifetime rule; p95 limit; maximum
limit; public Rust API; C ABI; or C++ surface changes.

## Alternatives

- Treat a never-observed `Unknown` launch as `Lost` after the authenticated peer
  disappears. Rejected because registry absence cannot distinguish an
  unobserved launch from an exact observed lifetime and would weaken replacement
  safety.
- Mark the launch observed through a new authenticated-peer native API. Rejected
  because the existing retained workspace handle already provides the required
  `Unknown` to `Live` transition without another trust boundary or FFI surface.
- Increase the finalization deadline or exclude shutdown from measured latency.
  Rejected because both hide the ownership defect and weaken unchanged accepted
  gates.
- Keep `FixtureController::finish` as a boolean and rely on `Drop` diagnostics.
  Rejected because callers cannot distinguish protocol, exact-exit, drain,
  identity, or cleanup-debt failures and can accept a measured sample before the
  hidden side effect is checked.

## Consequences

- Fixture startup may wait briefly for workspace registration after a peer has
  authenticated. It still uses one fixed launch deadline and performs no extra
  priming or retry on the qualification path.
- A fast exit after acceptance is classified `Lost` because that retained launch
  previously established `Live`; it cannot remain permanently `Unknown` and
  consume a finish deadline.
- macOS benchmark and fixture-test callers migrate to the typed finalization
  result. Windows fixture behavior, validators, public contracts, native shim,
  dependencies, packaging, and accepted budgets remain unchanged.
- Existing delayed cleanup remains finite and observable for genuine containment
  failures. Accepted watcher samples require explicit finalization success and
  terminal cleanup accounting.
- Historical Apple and Windows terminal-red evidence remains immutable. Fresh
  affected qualification on one integrated successor is still required before
  native watcher support can be reconsidered.

## Verification

- Deterministic fixture tests drive `Unknown` to `Live`, `Lost`, observation
  failure, and deadline exhaustion through the production acceptance seam and
  verify that only `Live` can return success without relaunch.
- Exact-exit tests prove authenticated lifetime is checked first, no launched
  probe occurs while it remains live, and only authenticated lost plus launched
  lost is stopped.
- Typed-finalization tests independently reject every incomplete acknowledgement,
  lifetime, bound, drain, identity, and cleanup-debt fact. Mutation of each
  non-compiler enforcement point must make its named test red.
- macOS fixture-controller, capture-start, retained-result, benchmark-harness,
  linkage, and target checks must pass with the private feature both enabled and
  disabled where applicable.
- The uninstrumented integrated Apple candidate must run a fresh ordered
  five-process cohort under ADR 0053's unchanged budgets. Any terminal red is
  retained without retry, replacement, exclusion, or budget change.
