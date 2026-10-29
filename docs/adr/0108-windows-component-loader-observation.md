# ADR 0108: Bounded Windows component loader observation

- Status: Accepted
- Date: 2026-10-27
- Current contract: [Windows bounded observer](../performance/methods/component.md#windows-bounded-observer)

## Context

Optional Windows component timing needs runtime provenance without debugger
suspension inside the sample interval. Windows [debugging events](https://learn.microsoft.com/en-us/windows/win32/debug/debugging-events)
suspend all threads of the affected process, including for ordinary thread
creation. Ignoring those events or subtracting their stops would not preserve the
measurement. Endpoint paths and hashes alone cannot identify transient loader
instances.

## Decision and rationale

Use an explicit benchmark-only, observer-on method: a fixed-storage
`LdrDllNotification` callback, source-ordered counter brackets, paired module
censuses and mapped-name/PE/open-file correlation. The collector retains
read-sharing-only file handles across the child and revalidates their identities
and bytes. Unknown or ambiguous instances refuse admission; the finite quotas
are not expanded to fit a run.

The [callback contract](https://learn.microsoft.com/en-us/windows/win32/devnotes/ldrdllnotification)
forbids calls into other modules and may change without notice. Resolve the API
explicitly, keep callback state alive until process exit and inspect each actual
linked callback before measurement. Source review still owns memory/loop
semantics; a machine-code allowlist is not a safety certificate for arbitrary
code. Use the existing owned-process runner, not another supervisor.

## Consequences

This supplies conditional observed-window and file/header correlation, not
backing-object or running-code identity. Registration history, delayed callbacks,
non-atomic census, final drain and post-serialization activity remain outside the
claim. Observer cost is included, never subtracted; default-off and list runs do
not register it. SDK behavior and other targets' observer methods are unchanged.

Each new apparatus needs fresh build, linked and runtime evidence. An applicable
method review, calibration and controls remain separate requirements for any
numerical claim. No historical failure is regraded and no quiet-host gate is
introduced. Contract regressions live in `test_windows_loader_callback.py`,
`test_windows_loader_observer.py`, `test_template_windows_loader.py` and
`template_matching_efficiency_exit.rs`; actual Windows verification remains
revision-bound.
