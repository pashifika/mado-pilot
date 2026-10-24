# ADR 0101: Current Apple native sanitizer host

- **Status:** Accepted — native-shim ASan library scenarios only
- **Date:** 2026-10-25
- **Resolves gate:** none; deployment, topology, input and performance scopes are unchanged
- **Supersedes:** CONTRIBUTING step 10's macOS 26.6.2 host selection for new ASan runs only

## Context

The current Apple workstation is macOS 27.0.1 (26A434), Apple M1 Pro, 32 GiB.
[ADR 0081](0081-current-apple-ocr-verification-cohort.md) adopted it for CPU OCR,
not capture/input. The earlier macOS 26.6.2 host is no longer available.
ASan verification must execute the current native ownership scenarios rather
than inherit a pass from the earlier OS or treat unavailable capture as a skip.

A fresh isolated build at product source `1e4d0b4` ran all 321 library tests
successfully on the current host. Cargo reported `mado_pilot_asan`; this makes
unavailable capture scenarios fail instead of skip. The native Objective-C shim
was compiled with `-fsanitize=address`. The compiler's ASan runtime was dynamically
linked from its resource directory and loaded at run time through an rpath. This
is native-shim instrumentation, not instrumentation of every Rust dependency.

## Decision

Use the exact current Apple host for new `CONTRIBUTING.md` step-10 ASan runs.
Accept the [recorded 321-test result](../evidence/phase-2-native/macos-27-asan-20261025.json)
only for its source, binary, host and library-scenario scope. An affected source
change still requires an applicability review or a new run. Missing permissions,
unavailable native capture or an ASan finding fail the check.

Skip refusal covers display capture and ownership scenarios. Some branches still
note an absent subject and pass:

- window-subject scenarios with no matching or producing window;
- multi-display, mixed-scale and seam scenarios with fewer than two suitable
  displays;
- the non-Retina scale note;
- idle-producer continuation after the first frame.

The recorded run did not retain passing-test output, so the execution of these
branches is not established. This result cannot support applicability for
window, topology or producer-progress changes. The run also does not satisfy the
historical coordinate/selection acceptance gate bound to the 26.6.2 host's display
inventory, and it records no display topology.

The macOS 26.5.2 deployment floor and historical macOS 26 evidence do not move.
This decision does not accept unexecuted mixed-scale movement, manual input,
watcher Lane B, performance budgets or every macOS 27 release. Those scopes keep
their own contracts and exact-host evidence. No permission or system setting
change is part of this decision.

## Alternatives

- Reuse the older host's ASan result: it does not cover the current native source
  and OS.
- Omit the check because that host is unavailable: loses an executable memory
  safety check despite the current host being able to run it.
- Treat one ASan suite as complete platform qualification: it does not exercise
  every topology, input route or workload.

## Consequences

The verification command, skip refusal, suite and sanitizer flags stay unchanged;
only the selected host for prospective runs changes. Build into a fresh dedicated
target directory, retain the Cargo-selected executable and hash it before and
after direct execution. Do not run Cargo again in a hash-pinned proof directory.

Integrators gain no new API, deployment requirement or implicit permission
behavior. The architecture baseline, verification instructions and release notes
state this narrow scope separately from earlier platform qualifications.

## Verification

The [source/command receipt](../evidence/phase-2-native/macos-27-asan-20261025.json)
and [retained run output](../evidence/phase-2-native/macos-27-asan-20261025.txt)
record 321 passed, zero failed, ignored or filtered tests, native exit zero and
unchanged executable SHA-256. The output is the launcher-captured combined output.
Its final `Wall time` line is the launcher's trailer, not test output, and the
retained bytes are unchanged.

After the run, the recorded applicability command found these paths matching
source: the four compiled workspace packages, `Cargo.toml`, `Cargo.lock` and
`rust-toolchain.toml`. A separate post-run check found the build inputs
`tools/setup-native.py` and `.cargo/config.toml` matching as well. These are source
applicability checks only. The runtime-loaded ASan dylib, Apple clang and Command
Line Tools path, version and hash identities are not bound, before or after the
run.

The test executable ran directly from Tern with existing Screen Recording
access. The sanitizer skip-refusal path is enforced in
`crates/platform/macos/src/scenarios.rs::skipped`; build configuration is enforced
by `crates/platform/macos/build.rs`. Source/host applicability and the separation
from broader platform qualification require release review.
