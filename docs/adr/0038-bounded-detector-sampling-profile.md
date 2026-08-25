# ADR 0038: Bounded detector sampling profile

- **Status:** Accepted
- **Date:** 2026-08-25
- **Resolves gate:** _none_; bounded-profile rows under `G-013` remain open until both release targets pass ADR 0039
- **Supersedes:** _none_; ADRs 0033, 0034, and 0037 remain unchanged for released native G-004

## Context

Released native G-004 applies DB736 preprocessing without a final large-input ceiling. A 3840×2160 frame therefore creates a detector tensor at source-scale dimensions, while a nominal intermediate 1280×720 transform can expand again under the short-side rule. Reusing the G-004 identity for a final-size cap would change detector pixels and invalidate revision-bound evidence.

The predeclared [qualification plan](../../rasen/changes/phase-3-1-bounded-detector-sampling-profile/evidence/qualification-plan.md) fixes a 1312×736 initial candidate, direct-resize pixel oracle, original-source recognizer rule, identical-input quality matrix, resource ceilings, and both-target acceptance policy before candidate inference. The [source baseline](../../rasen/changes/phase-3-1-bounded-detector-sampling-profile/evidence/source-baseline.md) binds the exact released source, toolchain, model components, vocabulary, and controlled runtime.

## Decision

Add one closed, explicitly selected bounded detector profile over the exact accepted G-004 component allocations. Its authoritative identifiers are:

- model and profile: `phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v1`;
- preprocessing: `rapidocr-ppocrv4-det-bgr-db736-fit-1312x736-linear-half-pixel-source-rec-v1`;
- model version, language profile, decoder, normalization, component lengths and SHA-256 digests, and 18,708-entry vocabulary identity: exactly those accepted by ADR 0033.

Claiming either accepted model or profile identifier requires an exact complete native G-004 tuple or exact complete bounded tuple. Cross-profile identifiers, preprocessing, model components, vocabulary, decoder, and normalization are rejected before session creation. Schema-v2 metadata carries the distinct identifiers through the existing closed fields; no optional flag or inferred default selects preprocessing.

For a selected profile, one internal preprocessing descriptor creates one checked detector plan before allocation. Native G-004 retains its released algorithm byte-for-byte. The bounded plan:

1. rejects zero or unrepresentable source dimensions;
2. computes native DB736 desired dimensions: scale both axes by `736 / min(source_width, source_height)` only when the short side is below 736, truncate the finite scaled values to `u32`, then independently round each dimension to the nearest multiple of 32 using ties-to-even;
3. if that desired size exceeds 1312×736, applies one shared fit factor `min(1312 / desired_width, 736 / desired_height)`, truncates both finite products, independently rounds each to the nearest multiple of 32 using ties-to-even, and clamps only downward to the greatest in-ceiling multiple when rounding would cross a ceiling;
4. rejects a dimension below 32, overflow, a non-multiple, an above-ceiling result, or checked tensor elements/bytes above the profile ceiling; and
5. records source/final dimensions, per-axis forward and inverse scale, three-channel float32 elements, and bytes in the plan.

The resize is one direct OpenCV 4.14.0 CPU `INTER_LINEAR` transform from the borrowed original BGRA source view to one final `CV_8UC4` matrix. Its contract is half-pixel centers, replicated edge samples, no explicit border fill, and OpenCV's fixed 8-bit linear interpolation rounding. After resize, the backend traverses the final matrix once, ignores alpha, writes complete planar B, G, and R channels in that order, and evaluates each byte as `((f32(v) / 255.0) - 0.5) / 0.5`. No intermediate 1280×720 image, retained detector image, analysis frame, or public analysis coordinate space exists.

Detector quadrilaterals are clipped and mapped into original source-view coordinates before candidate submission. Each axis preserves released G-004 floating operation order exactly: convert the detector `f32` coordinate to `f64`, divide by the checked final dimension, multiply by the source dimension, round ties-to-even, then clip. Multiplying by a precomputed inverse first is not equivalent at binary tie boundaries and is rejected by a regression oracle. Perspective crops for recognition are always read from the original mapped BGRA source using those source-coordinate quadrilaterals. Existing effective-region origin, `TransformSnapshot`, output-space conversion, and `FrameStamp` correlation then apply without caller scaling.

The bounded profile is available only through controlled explicit construction. `OnnxOcrBackend::open_accepted`, released facade defaults, C ABI 1.3, and C++ behavior continue to select only native G-004. Both profiles use one immutable model source, one session pair, two sessions, one admitted inference slot, exact ONNX Runtime 1.29.0 API 17 CPU loading, and no ambient search, download, provider fallback, or profile fallback.

Profile support is not accepted by this ADR. It requires the unchanged predeclared matrix to pass independently on approved Apple Silicon and Windows hosts, followed by ADR 0039 accepting or rejecting this exact ceiling and target budgets. Hosted CI is correctness/resource evidence only and cannot substitute release-host timing or resident-memory rows.

## Alternatives

- **Mutate the native G-004 preprocessing ID.** Rejected because it would silently relabel changed pixels, geometry, latency, and memory as released evidence.
- **Resize to 1280×720 and then run DB736.** Rejected because it allocates and resamples a second full image and still lets the short-side rule expand extreme-wide tensors.
- **Cap only the source mapping.** Rejected because detector tensor and inference work remain unbounded.
- **Recognize from the bounded detector image.** Rejected because it discards source detail before perspective cropping and changes recognizer pixels without evidence.
- **Fuse resize and planar conversion in custom unsafe/SIMD code.** Rejected until measurements show the reviewed OpenCV path is a bottleneck. It would add a second pixel implementation and a larger memory-safety proof before a stable oracle exists.
- **Select bounded preprocessing per request.** Rejected because a backend descriptor and session would no longer identify one immutable preprocessing contract.
- **Promote support from Apple, Windows CI, or native G-004 evidence alone.** Rejected because target runtime, allocator, quality, timing, and resident-memory behavior are profile- and revision-specific.

## Consequences

Integrators who need bounded detector work must choose the complete bounded identity through the controlled Rust construction path; doing nothing preserves native G-004. The profile limits detector tensor work, not source mapping size or original-source crop lifetime, so 4K mapping and native resident memory remain measured obligations.

The exact identifiers and direct pixels become compatibility facts. Changing ceiling, interpolation, half-pixel behavior, rounding, operation order, source-recognition rule, model component, vocabulary, decoder, or normalization requires a new identity and fresh evidence. A rejected 1312×736 candidate remains recorded rather than being silently tuned.

No dependency, license, runtime version, default constructor, C/C++ surface, model storage, session count, executor type, zone API, scheduler, provider, or network behavior changes. Public observations may report only the closed selected-profile class and bounded dimensions/counts/bytes; they never report paths, pixels, hashes, text, vocabulary, or raw native messages.

## Verification

- OCR authority and schema-v2 tests mutate every field in both closed tuples and exercise cross-profile mismatches.
- Checked planner tests cover 3840×2160, 2000×500, 2560×320, 960×540, small, odd, tie, exact-ceiling, zero, extreme, and overflow inputs.
- A fixed 4×3-to-3×2 pixel oracle fails on interpolation, pixel-center, border, channel, normalization-order, or extra-resize drift.
- Detector postprocessing tests cover edges, clipping, odd dimensions, output transforms, effective-region origin, and retained `FrameStamp`.
- Backend tests cover exact admission, graph/vocabulary mismatch, tensor/output bounds, busy, deadline/cancellation, native termination, recovery, close races, repeated close, and one-pair/two-session cleanup for both profiles.
- The qualification plan retains every Apple and Windows quality/resource row, exact source/executable/runtime/model/fixture identity, and failed sample without exclusion or retry.
- Final review requires an empty diff from base `dabfc3c27d634e19a073f68aa906712479eb1af2` for all frozen historical evidence paths and runs their existing drift/hard-budget registries unchanged.
