# ADR 0073: Link only required Apple OpenCV modules

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** _none_; `G-007`, `G-012`, and `G-013` remain open
- **Supersedes:** _none_; narrows the macOS development link selection under ADR 0066

## Context

The workspace enables only OpenCV `core`, `imgproc`, and `imgcodecs` bindings.
Native setup nevertheless exported `OPENCV_LINK_LIBS="+"`, preserving every
library advertised by `opencv4.pc`, and used that same unrestricted list for
its C++ probe. Binding selection did not constrain native linking.

The corrected OCR binding on source
`0fdba2a48651a1f1e8e9d0e26be80f5747758bf8` exited successfully with complete
measurement rows, but its 266 distinct non-system images exceeded the unchanged
256-image limit. The closure included 56 OpenCV, 59 VTK, and 79 Abseil images.
This remains a failed binding, not successful replay qualification.

## Decision

On macOS, setup explicitly selects the dynamic `opencv_core`, `opencv_imgproc`,
and `opencv_imgcodecs` libraries for both the Cargo environment and its C++
probe. Selected-root pkg-config checks still validate installation paths,
version, and compiler flags; its unrestricted library list no longer determines
the consumer's direct dependencies. Required shared-library transitive
dependencies remain loaded and observable.

Windows continues selecting the developer-owned versioned `opencv_world`
import library and matching DLL. This decision does not split that distribution,
change Rust features, add static or deferred loading, or acquire dependencies.
Native setup ownership and export rules in
[ADR 0066](0066-developer-owned-native-prerequisites.md) remain unchanged.

No image, output, memory, or timing ceiling is raised. Old artifacts and failed
runs remain pinned. Rebuilt artifacts have new identities and require their own
prospective execution decision; this ADR grants no model, capture, input,
permission, or qualification execution.

## Alternatives

- **Raise the image-count ceiling:** rejected. It would retain unnecessary eager
  dependencies rather than correct the link selection.
- **Filter extra images from evidence:** rejected. Every observed non-system
  dependency remains subject to the existing identity and count rules.
- **Repackage OpenCV or change binding features:** unnecessary. Existing shared
  modules already provide the required operations; binding features are narrow.

## Verification and limits

An actual fresh C++ consumer was compiled from an ordinary-OS environment
reconstructed solely from written GitHub exports. It passed color conversion,
PNG encode/decode, and pixel equality. Its 28 observed non-system images included
exactly the three selected OpenCV modules, with no VTK, Abseil, or OpenVINO.

That consumer is not the Rust OCR candidate. These observations prove the
exercised development setup and native behavior, not a candidate dependency
manifest, an OCR replay pass, a latency improvement, a numerical budget, or
minimum-host and redistribution support.
