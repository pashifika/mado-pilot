# ADR 0061: Separate native-watch privacy and environment validation

- **Status:** Accepted
- **Date:** 2026-09-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

The current-source Windows native template-watch process reached all 24 workloads,
then exited 101 before report publication with the bounded class
`privacy_violation`. A revision-bound differential reproduced the rejection while
changing only the Windows update revision from UBR 9168 to UBR 9278. A second
comparison showed that the approved host's exact byte-count hardware description
was also rejected in place of the validator's `32 GiB` text. Workload semantics,
progress, allocation growth, fixed tokens, profile construction, and report
emission were independently eliminated.

The shared validator treated fixture schema, privacy safety, and exact environment
compatibility as one `PrivacyFault`. Its only runtime caller erased every variant
to `privacy_violation`. Exact equality of mutable diagnostic text therefore
misclassified a safe serviced update as a privacy payload. The reproduced result,
cause selection, and independent review are retained by Rasen Change
`windows-native-template-watch-privacy-gate-repair`; the bounded result and cause
selection have SHA-256 identities
`4b96fb5c2bfb8afd5b22aa2f539c0f5a547ff229358a6d6139157150fc2725bc` and
`5a46f2a457409e6ec438ba38c7cf822a59d52b18aeef68cbcbb1cb5e7fb8ab9c`.

This counterexample invalidates the validator's exact Windows UBR and hardware
text admission rule. It does not change the immutable terminal-red process, its
unlaunched suffix, accepted budgets, or ADR 0060's `WITHHELD` support decision.

## Decision

Native watcher tracked-report admission uses two ordered checks behind one
validator interface:

1. Privacy and schema validation admits only fixed report fields, canonical
   identities, and finite bounded platform grammars. It rejects paths,
   credentials, captured/template content, OCR/input text, native identifiers,
   process inventories, free-form payloads, malformed or overflowing numbers,
   extra delimiters, and trailing data before report publication.
2. Environment compatibility independently checks approved host class, supported
   platform family, deployment target, SDK, toolchain, and backend. It returns a
   typed environment incompatibility distinct from schema and privacy failures.

For approved Windows host class `windows-i7-12700kf-32g`, Windows 11 Pro 25H2
build family 26200 and Windows SDK 10.0.26100.0 remain required. UBR is canonical
unsigned 32-bit decimal diagnostic provenance and is not an exact compatibility
gate. Hardware text is bounded structured provenance: it retains the fixed
processor description and admits either the historical `32 GiB` form or a
canonical unsigned 64-bit byte count. The approved host class remains the
hardware compatibility authority. Existing Apple allowlists and rejection rules
remain exact and unchanged.

The benchmark caller maps schema, privacy, and environment results to distinct
bounded failure classes and publishes no tracked report after any rejection. A
successful candidate publishes exactly one report only after semantic, privacy,
and environment checks pass. Every caller migrates to this result; no duplicate
exact-UBR or exact-hardware compatibility check remains.

## Alternatives

- Add UBR 9278 and the observed hardware string to the exact allowlist. Rejected
  because the next serviced update or equivalent bounded representation would
  fail for the same non-privacy reason.
- Accept arbitrary operating-system or hardware strings. Rejected because it
  would turn diagnostic provenance into a free-form payload channel.
- Keep one error type and change only the caller's panic text. Rejected because
  the validator would still erase the distinction between unsafe payload and a
  safe but unsupported environment.
- Suppress the failed report or omit provenance. Rejected because missing output
  is not green evidence and would weaken auditability.

## Consequences

- Qualification harness callers can distinguish `schema_violation`,
  `privacy_violation`, and `environment_incompatible`; no public watcher API,
  C ABI, C++ wrapper, capture/input behavior, dependency, or budget changes.
- A privacy-safe serviced Windows update can reach environment compatibility,
  while an unsupported edition, build family, SDK, target, toolchain, backend,
  or host class remains non-green.
- Windows grammar and classification tests become part of the shared testkit.
  Apple positive and adversarial cases prevent Windows rules from widening Apple
  admission.
- The repaired source requires fresh affected Windows qualification under a new
  protocol and process namespace. Existing Apple and Windows terminal-red
  evidence remains immutable, and native watcher support remains `WITHHELD`.

## Verification

- A pre-repair regression uses the exact bounded serviced-update report input and
  fails against source `030398e` at the shared validator seam.
- Shared testkit coverage exercises UBR and integer boundaries, canonical decimal
  form, supported and unsupported environments, Apple preservation, and every
  adversarial payload class named above.
- Mutation checks remove each non-compiler-checkable enforcement rule and prove a
  named regression becomes red.
- The uninstrumented native-template-watch report path must publish exactly one
  bounded report for accepted input and zero report bytes for rejected input.
- Focused Windows/shared tests, repository policy, privacy scanning, and
  complete-diff applicability must pass before independent review or any fresh
  formal qualification.
