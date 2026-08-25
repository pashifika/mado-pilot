# ADR 0040: Compound bounded-detector ceiling

- **Status:** Accepted; target budgets and final enforcement passed by ADR 0041
- **Date:** 2026-08-25
- **Resolves gate:** the profile-ceiling decision for the v0.3.1 explicit bounded OCR rows under `G-013`
- **Supersedes:** ADR 0038's rectangular-only `bounded-v1` candidate; ADRs 0033, 0034, 0037, and the ADR 0039 qualification formula remain unchanged

## Context

ADR 0038 predeclared an initial 1312×736 final-detector rectangle. Exact source `cff533826350be7dc991a50cc359733f01e654cc` ran five fresh native and five fresh bounded processes on each approved host without retry or exclusion. All bounded correctness/resource verdicts passed, but the fixed ADR 0039 formula rejected the candidate on Windows. Margin-derived ceilings exceeded fixed caps for 4K HUD, reference HUD, odd HUD, and dense tooltip; cold open also exceeded its derived cap. The exact rows remain in the Change evidence.

The caps are not relaxed. Evidence instead narrows detector work. A simple 1152×736 rectangle changed the extreme-wide detector from 1312×160 to 1152×128 and failed every retained Apple oracle. Unconditionally applying 6 MiB or 8 MiB tensor ceilings reduced work but changed the 960×540 detector from 1312×736 and failed the existing zero-tolerance dual-profile native HUD geometry contract. Five Windows unconditional-6-MiB processes are retained only as rejected diagnostics.

The first benchmark also constructed and retained all eight source frames before measuring any workload. That made process high-water include simultaneous unrelated fixture inventory. Qualification must construct one workload at a time so those frames are not concurrently live. Final process high-water measures the maximum reached across the complete workload sequence; it is not an isolated per-workload value. Prior RSS remains recorded but is not reused for candidate-v2 acceptance.

## Decision

Replace the unreleased `bounded-v1` tuple with one closed candidate-v2 tuple over the same immutable G-004 component bytes:

- model and profile: `phase-3-1-rapidocr-ppocrv4-det-v6-rec-small-bounded-v2`;
- preprocessing: `rapidocr-ppocrv4-det-bgr-db736-fit-1312x736-then-tensor6291456b-linear-half-pixel-source-rec-v2`;
- model version, language profile, decoder, normalization, component lengths and SHA-256 digests, and vocabulary identity: unchanged from ADR 0033.

The planner retains ADR 0038's checked native DB736 desired-size calculation and first 1312×736 aspect-preserving rectangular fit. It records whether that first fit was required.

- When the desired detector already fits the rectangle, no secondary tensor limit is applied. Reference 960×540 and odd 1001×563 inputs therefore remain 1312×736 with exactly the detector pixels and inverse geometry required by the existing native contract.
- When the desired detector required the rectangular fit and its three-channel float32 tensor exceeds 6,291,456 bytes, apply a second shared fit factor `sqrt(floor(6,291,456 / 12) / (width × height))` to the rectangular result. Truncate each finite scaled axis, floor each independently to a multiple of 32, and reject zero, below-32, overflow, non-finite, or above-limit results. Flooring rather than nearest rounding guarantees the tensor cannot cross the byte ceiling.
- When the rectangularly fitted tensor is already at most 6,291,456 bytes, preserve it. This retains 1312×320 wide-menu, 1312×160 extreme-wide, and 576×736 mission-region work.

The fixed qualification workloads therefore expect bounded detector dimensions:

| Workload | Final detector | Tensor bytes |
|---|---:|---:|
| 3840×2160 HUD | 960×512 | 5,898,240 |
| 2000×500 wide menu | 1312×320 | 5,038,080 |
| 2560×320 extreme-wide status | 1312×160 | 2,519,040 |
| 960×540 HUD | 1312×736 | 11,587,584 |
| 1001×563 odd HUD | 1312×736 | 11,587,584 |
| 1440×720 dense tooltip | 1024×480 | 5,898,240 |
| 563×720 mission region | 576×736 | 5,087,232 |
| 3840×2160 blank | 960×512 | 5,898,240 |

The profile's absolute detector rectangle remains 1312×736/11,587,584 bytes because reference-size desired work does not invoke the second fit. Backend facts report that absolute bound; per-call observations report the actual final dimensions and bytes. The 6 MiB value is an additional large-desired-input rule bound by the preprocessing identity, not a relabeling of the absolute fact.

The benchmark constructs, hashes, measures, and drops one workload fixture at a time. It reconstructs one reference fixture for cancellation after timed workloads. This changes no production frame ownership or OCR pixels; it prevents unrelated source frames from being simultaneously live. Target RSS APIs expose process-lifetime high-water rather than resettable workload peaks. Schema version 3 therefore names each diagnostic checkpoint `process_peak_resident_bytes_after_workload`; later values may include earlier peaks. Only the final report-level `peak_resident_bytes` is an independent process-budget input.

All ADR 0038 direct-resize, interpolation, half-pixel, channel, normalization, divide-then-multiply inverse geometry, original-source recognition, explicit selection, one-session-pair, controlled-runtime, privacy, and non-default rules remain unchanged.

## Alternatives

- **Relax Windows latency or RSS caps.** Rejected because ADR 0039 fixed them before target evidence and the user priority is performance and memory safety.
- **Accept the rectangular candidate because quality passed.** Rejected because the fixed cross-target performance gate is part of profile qualification.
- **Use a smaller rectangle globally.** Rejected by the extreme-wide oracle at 1152×128.
- **Apply 6 MiB to every source.** Rejected because it changes reference detector geometry and fails the zero-tolerance native contract.
- **Keep all fixtures resident and subtract their bytes.** Rejected because allocator/OpenCV/ONNX high-water is not a linear accounting quantity; constructing frames lazily and budgeting the final process high-water is simpler and reproducible.
- **Fuse resize and planar conversion.** Deferred. Current evidence first removes excess inference work without adding a second unsafe pixel implementation.

## Consequences

`bounded-v1` remains rejected evidence and is not an alias. Candidate v2 passed exact-source Apple and Windows precursors, ADR 0041 accepts fixed-formula target budgets, and strict final enforcement passes 5/5 on each approved host.

The secondary rule intentionally preserves reference-size detector pixels while reducing oversized and dense inputs. Source mapping and original-source recognizer crops remain uncapped obligations. Lazy benchmark fixture lifetime reduces measurement contamination but does not change public frame lifetime or producer behavior.

No released native G-004 identifier, default constructor, C ABI/C++ surface, dependency, runtime/model byte, provider, model storage, session count, network behavior, or frozen Phase 3 evidence changes.

## Verification

- Planner tests cover fitted high-area, fitted low-area, reference, odd, small, tie, extreme, exact-rectangle, zero, and overflow cases and assert the secondary byte bound where applicable.
- The existing fixed direct-pixel and divide-then-multiply inverse-geometry tests remain unchanged.
- The ignored native backend contract passes both profiles, including zero-tolerance reference HUD geometry, cancellation, recovery, and close races.
- Exact schema-v3 source `ce658b3` passed five fresh bounded and five fresh native processes on the approved Apple M1 Pro with zero oracle failure/growth, complete final/cumulative RSS, and every formula-derived Apple candidate within its unchanged cap.
- Approved Windows fresh precursor and both-target final enforcement remain required under ADR 0039.
- Frozen Phase 3 evidence paths must remain byte-identical to `dabfc3c27d634e19a073f68aa906712479eb1af2`.