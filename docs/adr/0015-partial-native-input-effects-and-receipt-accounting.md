# ADR 0015: Account for Partial Native Input Effects

- **Status:** Accepted
- **Date:** 2026-08-01
- **Resolves gate:** _none_
- **Supersedes:** _none_

## Context

One logical input event can require several native operations. Windows text uses
Unicode key-down/key-up records for every UTF-16 unit, and a two-axis scroll uses
one vertical and one horizontal mouse record. Microsoft documents that
[`SendInput`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput)
returns the number of records it inserted, which can be less than the number
requested, and does not identify UIPI through its return value or last error.
Likewise,
[`SendMessageTimeoutW`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw)
reports failure or timeout as zero but supplies no cancellation acknowledgement
from the receiver. Once dispatch has begun, a missing application acknowledgement
therefore does not prove that the receiver observed no part of the operation.

The original input-receipt wording treated `Partial` as requiring at least one
complete logical event and `Unexecuted` as safe to retry because nothing happened.
Those two statements cannot both remain true after a short native insertion or an
unacknowledged dispatch of the first logical event.

## Decision

`InputReceipt::delivered` counts only logical events proven complete. `Partial`
means that some input may have reached the target and is permitted with
`delivered == 0` when the first logical event may have taken effect only in part.
`Unexecuted` is reserved for failures proven to occur before any event effect.

The Windows input driver carries an internal before-event/during-event failure
classification. A short nonzero `SendInput` insertion and an attempted fixture
dispatch without the expected acknowledgement are during-event failures. A proven
UIPI refusal before insertion, static admission failure, cancellation, deadline, or
geometry failure before native delivery remains before-event. No fallback is tried
after a during-event failure.

Sequence cleanup continues to derive owned pressed state from complete logical
events. Every supported button or key press is one native record; the multi-record
events are scroll and text, which add no contract-owned pressed state. If Unicode
delivery stops after a down record, the Windows driver additionally attempts one
bounded matching key-up before reporting the partial event.

## Alternatives

**Report zero complete events as `Unexecuted`.** Rejected because the receipt would
claim that retry is safe after Windows reported that part of an event was inserted.

**Count a partly inserted event as delivered.** Rejected because `last_completed`
would then claim that the requested text or two-axis scroll completed when only a
prefix may have reached the system.

**Reject every event that expands to multiple native records.** Rejected because it
would remove Unicode text and two-axis scrolling even though Windows exposes exact
native progress sufficient to report their non-atomic failure honestly.

**Add native-record counts to the public receipt.** Deferred because such counts
are platform-specific and do not tell a caller which part of a Unicode scalar or
scroll event is semantically usable. The complete-event count plus retry-safe
outcome distinction is the portable information callers need.

## Consequences

- Callers must treat every `Partial` receipt as non-retry-safe, including one whose
  complete-event count is zero.
- Existing complete-event accounting, `last_completed`, cleanup counts, status
  values, and ABI enum values do not change.
- Adapter implementations must distinguish a proven pre-effect failure from an
  uncertain or observed partial effect whenever their platform operation is not
  atomic.
- A platform that cannot make that distinction must choose `Partial`; conservatism
  can prevent an automatic retry but cannot duplicate user input.
- The shared receipt documentation, Windows controller seam, Windows delivery
  backend, and their tests change together in this Change.

## Verification

- `mado-pilot-input` tests assert that `Partial` can carry zero complete events,
  no `last_completed`, and a selected delivery mechanism.
- Windows controller tests inject a during-event failure on the first event and
  assert `Partial`, zero complete events, and no fallback. The corresponding
  before-event paths continue to assert `Unexecuted`.
- Windows native delivery compares the `SendInput` return count with the requested
  record count and requires the dedicated fixture acknowledgement.
- The dedicated fixture integration test exercises complete synchronous
  background delivery, while deterministic tests cover the short/uncertain branch
  without injecting system input into a developer desktop.
