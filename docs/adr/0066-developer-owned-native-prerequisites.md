# ADR 0066: Developers own native development prerequisites

- **Status:** Accepted
- **Date:** 2026-09-05
- **Resolves gate:** _none_; `G-007` and `G-012` remain open
- **Supersedes:** _none_

## Context

The user amended the current Phase 5 Change to configure developer-owned native
installations, not qualify bundled deployment candidates. This source-only
project redistributes no native libraries or models. The original
[five-candidate protocol](../evidence/native-release-profiles/protocol.json) and
[supporting results](../evidence/native-release-profiles/supporting-execution-01.json)
remain frozen historical evidence. The ordinary direct Rust consumer's
missing-OpenCV pre-entry failure remains a failure under that original protocol;
the private host-load comparison did not repair it.

## Decision

Developers acquire OpenCV 4, loadable libclang, and the native compiler/SDK from
upstream distributions or package managers, then explicitly run
`tools/setup-native.py` to configure and check those existing installations.
Missing or incompatible development prerequisites may fail setup or build.
Removing eagerly linked OpenCV after setup invalidates the environment and may
prevent process entry. No typed-recovery guarantee is made for that violation,
so this change adds neither a private deferred-load bridge nor a static workaround.

The Python 3.13+ interface is `tools/setup-native.py [--opencv-root DIR]
[--libclang-path DIR] [--github-env FILE --github-path FILE]
[-- COMMAND ARG...]`. No-command mode prints validated target/version/root and
environment JSON. Command mode invokes one executable with a child-scoped
environment and propagates failure, without shell evaluation or parent-shell
mutation. macOS may discover `opencv@4` and the selected Xcode Command Line Tools;
Windows requires both roots and an x64 MSVC developer prompt. Chosen roots take
precedence over conflicting OpenCV discovery inputs. Explicit GitHub exports
require both file options, successful validation, and safe environment-file
values.
macOS command mode prepends selected OpenCV libraries while retaining unrelated
inherited loader paths. JSON/CI output does not disclose those inherited entries.
An explicit GitHub export refuses additional inherited `DYLD_LIBRARY_PATH`
entries rather than erase or publish them; use child-command mode without export
options. Repeated entries naming only the selected directory remain admissible.

Neither setup nor product runtime downloads, installs, elevates, or changes the
registry or global shell configuration. Existing ORT/model caller-selected
canonical paths, exact runtime/profile identities, model length/hash checks,
restricted loading, and typed absence/mismatch outcomes remain mandatory. So do
platform capability/permission checks and existing provider policy. No production
Rust API, ABI, or loader implementation changes.

## Alternatives

- **Private deferred OpenCV bridge:** rejected for this scope. It adds a native
  loading/lifetime boundary solely to recover from a violated development
  prerequisite, not a required runtime capability.
- **Static-link workaround:** rejected; it changes native build, closure, and
  licensing obligations without addressing the chosen setup responsibility.
- **Implicit installation or persistent shell configuration:** rejected. The
  developer must choose acquisition and explicitly invoke configuration; product
  runtime cannot repair the machine.

## Consequences

Developers must maintain the selected installations and run native commands in
the configured child environment. A successful setup does not prove a complete
redistributable closure, future library availability, or clean/minimum-host and
signing qualification. The existing OpenCV build already enforces native build
prerequisites; no extra Rust build guard is added just to repackage its errors.

[Contributor commands](../../CONTRIBUTING.md#native-development-prerequisites),
[architecture](../architecture.md#native-development-prerequisite-ownership),
[dependency policy](../third-party-dependencies.md#opencv),
[gate status](../validation-gates.md#g-007), and the current Change adopt this
amendment. The Direction's higher-level full-release/Slice acceptance is not
rewritten. This incremental decision does not establish full Slice acceptance,
merge readiness, a native release, installer, static artifact, or broader support.

## Verification

Source `dfd302fcd9f44fc4b637a1a2c72b5eb6a0216fc8` passed both hosted native
jobs, repository policy, and branch policy. Actual macOS and Windows setup,
configured commands, prerequisite refusals, literal arguments, and explicit
exports were exercised. Windows rebuilt from an ordinary OS baseline using
only the exported native settings; macOS additionally proved caller-owned
dylib loading, private-export refusal, and selected-directory alias handling.

The [source-bound execution record](../evidence/native-release-profiles/developer-setup-execution-01.json)
retains the initial SDK/MSVC failures, their corrections, regression mutation
checks, and independent Codex review. Seventeen frozen profile/G-004/release
files retain their original bytes, and the G-004 validator passes. The new
development-host results do not promote the historical qualification records
or close the broader gates.
