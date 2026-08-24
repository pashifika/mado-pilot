# ADR 0036: Default OCR composition and ABI prefix

- **Status:** Accepted
- **Date:** 2026-08-24
- **Resolves gate:** _none_ (`G-013` remains open)
- **Supersedes:** the package-required default-flow assumption in ADR 0035; the private-fixture boundary remains unchanged

## Context

The accepted G-004 model, bounded ONNX CPU backend, and ABI 1.3 OCR calls now exist together. The integration design requires one explicit, fallback-free production composition root, but it does not define how a blocking model/runtime load receives the caller's operation context or how C and C++ select the composition.

Two existing seams cannot implement that contract safely:

1. `replay_engine`, `windows_engine`, and `macos_engine` receive no construction operation. Storing default prerequisites in their existing request objects would make the 25,979,900-byte model read and native session creation ignore the caller's deadline and cancellation.
2. ABI 1.3 OCR requests currently require an `AssetPackage` so the boundary can resolve a model identity. A default ONNX backend already retains the exact detector and recognizer source. Loading the same two components into a package solely to name that identity retains a second 25,979,900-byte Rust allocation before native session memory. The duplication follows directly from `OnnxOcrBackend` retaining `OcrModelSource` session inputs and `AssetPackage` retaining another `OcrModelSource`; it is not an inferred allocator overhead.

An initial implementation attempted to append two `madopilot_str_t` views to the frozen ABI 1.2 `madopilot_engine_options_t`. The independent frozen-header probe disproved that design: the released structure is 16 bytes with alignment 4, while the appended pointer views produce 48 bytes with alignment 8. A valid ABI 1.2 caller may therefore supply only 4-byte alignment, which the new Rust type could not require as 8-byte aligned. The probe failed before old-header execution. Weakening its alignment oracle would hide a real caller contract change, so the attempt was removed rather than accepted.

## Decision

Add explicit facade constructors for the default OCR composition. They accept:

- a caller-selected absolute model root;
- a caller-selected canonical absolute ONNX Runtime path; and
- the caller's executor-neutral `OperationContext`.

The composition reads only the two fixed G-004 relative model paths, requires their exact sizes and SHA-256 identities before session publication, opens exactly ONNX Runtime 1.29.0 through API 17, and selects only the built-in CPU provider. It performs no environment lookup, directory search, download, retry, provider fallback, or alternate-model selection. Existing constructors continue to mean “no default OCR”, and the explicit backend injection seam remains for Rust integration and deterministic tests.

ABI 1.3 keeps its 592-byte ABI 1.2 table prefix unchanged and appends `engine_create_with_default_ocr` at offset 640, producing a 648-byte complete table. The entry accepts the unchanged 16-byte `madopilot_engine_options_t`, a new mandatory 40-byte `madopilot_default_ocr_options_t` carrying two borrowed UTF-8 path views, and the caller's operation. A distinct entry is permanent surface, but it preserves the released structure's size and alignment and keeps all prerequisite ownership explicit.

`madopilot_build_info_t` appends static default backend, runtime, model, and profile identities, and engine capabilities report whether OCR is configured. The released build-info alignment was already 8, so its size-versioned suffix preserves the frozen prefix. ABI 1.2 callers continue to negotiate 592 bytes, pass their exact released structures, and observe no OCR backend.

For a session carrying the integrated default backend, a C OCR request may omit `package` and must still name the selected model/backend identities exactly. The boundary borrows the already configured model identity; it does not allocate or load model bytes again. A package remains required for every non-default backend, including the private deterministic fixture. This is the only ADR 0035 behavior superseded here: its ownership, immutable result, panic containment, table ordering, frozen older prefixes, and private-fixture isolation remain unchanged.

## Alternatives

- **Store prerequisites in existing Rust engine requests.** Rejected because construction would have no caller deadline or cancellation.
- **Append default prerequisites to `madopilot_engine_options_t`.** Rejected by the compiled frozen-header probe: pointer-length views change the released type's alignment from 4 to 8. Packing, split pointer words, inline path buffers, or weakening the checker would trade a visible ABI defect for less portable or less safe representation.
- **Use process environment variables.** Rejected because process state becomes an ambient selection/search mechanism and cannot express per-call ownership or validation.
- **Require an asset package in the default C/C++ flow.** Rejected because it retains a second complete model source only to prove an identity already fixed and retained by the selected backend.
- **Let a null package mean any selected backend.** Rejected because caller-injected or private-fixture policy would become implicit. The shortcut is restricted to the exact integrated backend and accepted model identity.

## Consequences

- Default OCR construction is explicitly blocking, cancellable, deadline-bound, and atomic with engine publication.
- Missing model files or runtime return typed `Unsupported`; invalid model bytes return `AssetInvalid`; malformed or non-canonical configuration returns `InvalidArgument`. Retained details name only public controlled relative paths and expected identities, never host paths or native loader messages.
- Rust, C, and C++ use one production composition root without duplicating model bytes at the public-language boundary.
- The ONNX runtime library still remains process-global until process exit under ADR 0034; engine close releases sessions and model allocations, not that library handle.
- ABI 1.0 and 1.2 table, structure, numeric, alignment, and behavioral prefixes remain unchanged. ABI 1.3 finalizes the new 40-byte prerequisite structure, the build-info suffix, and the default-construction entry at offset 640 for a 648-byte table.

## Verification

- Rust tests cover fixed-path loading, exact model identity, prerequisite failure classes, cancellation/deadline propagation, configured descriptor visibility, and repeated close.
- C layout and prefix tests prove the 16-byte/alignment-4 ABI 1.2 options structure remains exact, the standalone default options are 40 bytes/alignment 8, and the complete ABI 1.3 table is 648 bytes.
- C and C++ common flows negotiate the new entry, read the reported identities, recognize without a duplicate model package, and retain/release results in varied order.
- Native Windows and Apple Silicon jobs load the exact controlled runtime/model identity and run the same accepted fixture rows.
