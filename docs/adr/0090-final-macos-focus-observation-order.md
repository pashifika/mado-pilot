# ADR 0090: Final macOS focus observation order

- **Status:** Accepted
- **Date:** 2026-10-21
- **Resolves gate:** _none_
- **Supersedes:** [ADR 0016](0016-macos-input-delivery-surface-and-focus-authority.md)
  only for the ordering of the final retained-window inventory and Accessibility
  focus observation

## Context

`System` window input requires focus; `ProcessDirected` honors an explicit
`RequireFocused` predicate without activation. A retained window can keep the
same identity and geometry while its owner loses the foreground during a bounded
ScreenCaptureKit inventory wait. Inventory after the last Accessibility read
therefore cannot establish current focus from unchanged bounds alone.

The shared observer in
[`madopilot_macos_shim.m`](../../crates/platform/macos/native/madopilot_macos_shim.m)
joins retained logical-window authority with public Accessibility geometry. Its
final observation must cover focus changes during the last inventory wait. This
ordering requirement is source-derived; it is not a native reproduction or a
new qualification result.

## Decision

The macOS adapter completes its final retained-window inventory before the final
Accessibility frontmost, focused-window, and focused-window-bounds observations.
Both input routes reuse that inventory's returned bounds for final geometry
validation without another blocking inventory read after accepted focus.

## Alternatives

- Inventory after the final Accessibility predicate: unchanged identity and
  geometry cannot detect a foreground change during that wait.
- Another inventory after accepted focus: recreates the same unobserved interval
  rather than strengthening focus authority.
- An OS-atomic focus-and-post guarantee: the separate public observation and
  posting APIs provide no such guarantee.

## Consequences

- A focus change observed during final inventory refuses the pending event;
  accepted focus is a last observation, not atomic delivery or application
  consumption evidence.
- Public Rust signatures and C ABI layouts are unchanged. The internal shim
  version is 26; its focus result carries the authoritative bounds back to Rust.
- Retained-window identity, one-to-one Accessibility geometry joining,
  authorization, activation, cleanup, geometry policies, and fallback rules keep
  their existing responsibilities. No permission or private window identifier is
  added.
- ADR 0016 retains its other rules and historical facts. ADR 0029's process scope,
  historical support decisions, source identities, and performance measurements
  are unchanged. `RequireFocused` remains outside the one-inventory-read latency
  profile.
- The architecture baseline and macOS verification guide describe this ordering;
  future observation changes must preserve it or record a scoped successor.

## Verification

Run these permission-independent regression tests through the existing macOS
library-test procedure:

- `system_required_focus_observes_changes_during_the_final_inventory` in
  [`native_input/tests.rs`](../../crates/platform/macos/src/native_input/tests.rs)
  checks zero-post `FocusRequired` and stable-focus acceptance through the shared
  native observation seam.
- `process_required_focus_observes_changes_during_the_final_inventory` in
  [`shim.rs`](../../crates/platform/macos/src/shim.rs) checks the same ordering
  inside process-directed final commitment.

The separately authorized ignored rows in the
[macOS verification guide](../macos-input-verification.md#bounded-focus-and-raw-geometry-regressions)
remain a pending current-source native gate. Deterministic tests do not qualify
capture, physical input, the current host, or statistical performance; historical
qualification artifacts remain bound to their recorded revisions.
