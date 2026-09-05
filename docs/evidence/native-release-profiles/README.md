# Native release-profile qualification

## Current scope: developer-owned prerequisites

**User-approved amendment, 2026-09-05:**
[ADR 0066](../../adr/0066-developer-owned-native-prerequisites.md) replaces this
Change's bundled/host candidate and private-bridge acceptance with explicit
development setup. Developers acquire OpenCV 4, libclang, and the native
compiler/SDK separately from upstream or package managers, then run
`tools/setup-native.py` with Python 3.13+. This source-only project redistributes
no native library or model; neither setup nor product runtime downloads or
installs dependencies.

The interface is `tools/setup-native.py [--opencv-root DIR] [--libclang-path DIR]
[--github-env FILE --github-path FILE] [-- COMMAND ARG...]`. No-command mode
validates and prints target/version/root/environment JSON; command mode runs an
executable with the configured child environment and propagates failure.
Windows requires both roots and an x64 MSVC developer prompt; macOS may discover
`opencv@4` and the selected Xcode Command Line Tools. CI export requires the
paired file options and successful validation. See the complete
[acquisition and invocation instructions](../../../CONTRIBUTING.md#native-development-prerequisites).

An incomplete development environment may fail setup or build. Removing eagerly
linked OpenCV after setup invalidates the environment and can prevent process
entry; no private deferred-load bridge or static-link workaround is required.
Existing caller-controlled ORT/model canonical-path, length/hash, typed runtime
refusal, provider, and platform capability contracts remain mandatory.
Current setup implementation and native verification remain pending integration
review; none of the historical observations below is a setup pass.

## Frozen qualification method (historical)

[`protocol.json`](protocol.json) (`native-release-profile-v1`) fixes the original
five candidates and their old acceptance envelope.
[`baseline.json`](baseline.json) binds their initial product, Direction, and
development builder. The protocol, inventories, and result JSON remain
byte-for-byte historical; their required rows are superseded for the amended
Change, not relaxed into passes. The following procedure still describes that
original apparatus, not current setup acceptance. Its own protocol checks remain
unchanged. No profile was selected or installable artifact published.

### Run the historical procedure

CI uses Python 3.14. The procedure supports Python 3.11+ on POSIX and 3.13+ on
Windows, where private-directory creation requires owner/admin-only access.
No Python package installation, PowerShell orchestration,
network access, desktop permission, or real input is required by these tools.
The manifest is reviewed executable configuration, not untrusted data: its
commands run with the caller's authority. Do not run an unreviewed manifest.

```sh
python3 tools/native-release-profile/qualify.py \
  --candidate cpu-host-macos \
  --manifest /absolute/reviewed-manifest.json \
  --manifest-sha256 <reviewed-sha256> \
  --attempt <new-attempt-id> \
  --root builder=/absolute/builder-files \
  --root models=/absolute/model-root \
  --output /absolute/private-attempts
```

Windows uses `python` and native absolute paths with the same arguments. Each
`--root` binds one explicit directory alias. Arguments and approved environment
values expand `{alias}`, `{stage}`, and `{scratch}` without a shell. Staged
executables retain their relative native-library layout and are reverified before
invocation. External executables are copied into private scratch and verified
there by SHA-256, so later builder-file replacement cannot change their bytes.
The environment does not inherit ambient library search paths; a decoy row must
name its search path.

An attempt exclusively creates `<output>/<candidate>/<attempt>`. It records the
exact reviewed manifest bytes before staging, rejects duplicate JSON keys, and
retains `result.json`, bounded row logs, nonzero
outcomes, skipped rows, and cleanup failures. A duplicate attempt is refused even
if the earlier process never reached its final record. Changed source, oracle,
layout, dependency, feature, or executable bytes need a new manifest digest and
attempt. Never edit an earlier result into a successor pass.

The process record distinguishes ordinary exit, timeout, output overflow, launch
failure, and cleanup failure. Required output must occur exactly once as a whole
line in addition to the expected exit code. Exit zero without that observation
fails. Any failed mandatory row or cleanup prevents procedure success; explicit
unexecuted rows also prevent success. `qualification` always remains
`not-selected`: successful commands do not establish clean admission, complete
candidate coverage, license approval, or an ADR decision.

Raw logs, commands, and source roots are private local evidence. Do not commit
attempt directories. Publish only separately reviewed content-free observations
and artifact identities; exclude images, recognized/input text, credentials,
process paths, and unrelated desktop metadata.
Attempt directories and files restrict access at creation, independently of the
caller's umask. Interruptions retain failure records and never become passes.

### Manifest contract

The version-one object has exactly these fields:

| Field | Contract |
|---|---|
| `schema_version` | Integer `1` |
| `candidate` | One identifier from `protocol.json` |
| `source_commit`, `source_tree` | Full 40-character source identities |
| `features` | Exact candidate feature list; no fixture/CoreML/all-features substitution |
| `artifacts` | Nonempty exact file declarations described below |
| `rows` | Unique attempt-local commands or explicit unexecuted rows |
| `admission` | `kind` (`development-host` or `clean-consumer`) and `record_sha256` |

A clean-consumer declaration additionally requires `--admission` with the exact
reviewed record digest. This binds an external admission record; the procedure
cannot independently prove its truth. A development-host record uses a null
digest and never qualifies a clean consumer.

Each artifact declares `id`, `root`, `path`, `destination`, `category`,
`ownership`, `sha256`, `bytes`, `source`, `version`, `license`, `notice`, and
`redistribution`. Roots are aliases; source and destination paths are safe
relative POSIX names. `ownership` is `product`, `bundled`, or `host`.
Host-provided files use a null destination and are verified without copying.
`redistribution` is `approved`, `host-only`, or `unresolved`; unresolved bundling
is rejected. Models and CUDA/cuDNN stay host-provided. A complete native closure
and notice review cannot be inferred from a set of self-declared files.

Each row has `id`, `argv`, `executable_sha256`, `environment`, `expected_exit`,
`required_stdout`, and `unexecuted_reason`. A runnable row uses null
`unexecuted_reason`. An unexecuted row instead names a content-free reason token,
uses null command/digest/exit/output fields, and an empty environment. All
mandatory cases in `protocol.json` remain obligations even when a supporting
manifest exercises only a subset.

### Bounds and accounting

The fixed envelope is 512 declared files, 4 GiB aggregate declared file bytes
including host inputs, 64 rows, 120 seconds execution per row, 5 seconds owned
process cleanup, 1 MiB combined raw output per row, and a 1 MiB manifest. These are
apparatus ceilings, not product latency or memory claims. Commands must not write
outside owned scratch or spawn detached processes that escape ownership. The
runner is not a sandbox or a general disk-quota mechanism.
Invalid UTF-8 is replaced after byte-bounded capture; encoded text logs can expand
to at most three times that raw-byte limit.

POSIX children run in an owned process group. Windows children are assigned to an
owned Job before execution. No process-name scan, unrelated PID termination,
registry/system-directory edit, elevation, or input-based cleanup is allowed.
Source roots are read-only; cleanup removes only attempt-owned scratch. Native
libraries remain loaded while function pointers or ORT state can reference them.

Byte categories are separate: Rust consumer, shared library, Windows import
library, headers, native payload, notices, other consumers, fixtures, models,
expanded package, compressed package, and host-supplied payload. Category totals
deduplicate repeated canonical source identities within that category. Expanded
package bytes count every installed package path, not unique content hashes.
Host-supplied bytes count host files without pretending they are shipped.
Compressed bytes mean the actual deterministic ZIP length, including ZIP
metadata. A library size is never compared with a complete bundle as the same
metric. Missing archives or inventories remain unmeasured, not zero-byte claims.

Staging rejects links, reparse points, special files, traversal, aliases and
colliding destinations, changed bytes, and missing/unapproved redistribution
information. It hashes and copies in bounded chunks, never reads a whole model
or native payload into memory, and verifies the file that was actually opened.

### Native consumers and loader experiments

Build the ordinary libraries first, with no private fixture features:

```sh
cargo build --locked --release -p mado-pilot -p mado-pilot-capi
python3 tools/native-release-profile/check_consumers.py \
  --root . --library-dir target/release --output target/profile-consumers-run1 \
  --model-root "$MADO_PILOT_G004_MODEL_ROOT" \
  --runtime "$MADO_PILOT_ONNX_RUNTIME"
```

On Windows, run inside the MSVC x64 developer environment and replace the shell
variable notation with the native shell's equivalent. Also pass `--opencv-runtime`
with the controlled `opencv_world4140.dll` path; this option is Windows-only.
The driver invokes `cl /utf-8` directly and builds external Rust facade, current
C/C++, and immutable ABI 1.0/1.2/1.3/1.4 consumers. It exercises offline matching,
blank-image CPU OCR,
frame correlation, interruption, and ownership checks. Its development-host
results are supporting evidence only. CI runs the deterministic procedure tests
on all three policy/native hosts and the consumer lane on both release targets.

`inspect_native.py --file <artifact> --tool <absolute-otool-or-dumpbin> --output
<new-json>` records native architecture/import metadata and explicitly labels
actual loading as unobserved. PE imports do not enumerate all dynamic CUDA
requirements. Mach-O load commands alone do not exclude runtime substitution.

`host_load.c` is a private comparison mechanism, not a changed product API. It
loads the exact `MADO_PROFILE_LIBRARY` with immediate local resolution on macOS.
On Windows it first loads the explicit `MADO_PROFILE_OPENCV_RUNTIME`, then the
candidate, using only DLL-directory/System32 resolution for both. Missing preload
inputs refuse before candidate loading. The temporary preload reference is
released after candidate loading; the candidate retains its own dependency.
C/C++ probes use
`qualification_get_api`; the host, not MadoPilot, owns a failed load's
`UNSUPPORTED` status. Its optional Rust bootstrap invokes the same direct-facade
consumer work inside a private deferred module, with no Rust object crossing the
module boundary. Neither arrangement makes an ordinary directly linked Rust
executable safely load without OpenCV. Qualification must keep that distinction
and reject any approach that merely substitutes the helper for a required public
consumer. Loaded-module paths are private attempt logs, not public diagnostics.
Successful probes require a complete process-exit snapshot containing the
candidate and, on Windows, the requested OpenCV module after the temporary
reference is released. This does not observe earlier unloads or prove the whole
runtime closure.

### Supporting execution checkpoint (historical)

[`supporting-execution-01.json`](supporting-execution-01.json) binds the green CI
and native consumer results to `c34fcc01c2c76e605272a6d3dd22cc817d6d5030`,
retains earlier failures, and records the explicit preload and Nix experiments.
The Windows lanes passed 28 rows each; macOS passed 26. A wrong retained-library
requirement rejects an otherwise successful child.

The Nix C++ run passed with Homebrew reads denied, but remains development-host
evidence. Adding weak-library flags to the ordinary Rust consumer left strong
load commands intact and still aborted before entry when OpenCV was unavailable.
Neither result completes the backend-only deferred bridge or admits a profile.

## Broader release gates remain open

`G-007` still requires evidence for any future native deployment profile;
`G-012` still requires the published target/feature/capability/size/default matrix.
Developer setup and a successful native build are not clean-consumer admission.
An Apple 26.6.2 run does not qualify the encoded 26.5.2 floor; ad-hoc local signing
is not distribution signing or notarization. No broader Windows, CUDA, CoreML,
or native capability support is inferred.

The Direction's higher-level full-release/Slice acceptance has not been
rewritten by this incremental amendment. Setup completion cannot assert that
result, merge readiness, a native release, an installer, or gate closure.
`G-008`, future `G-006`, and any new `G-013` claim remain separate. Historical
failures, product defaults, public API/ABI, source-only release scope, and existing
support qualifications retain their original authority.
