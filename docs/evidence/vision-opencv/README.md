# OpenCV CPU backend: cross-target matching evidence

This directory holds the measurements behind the score tolerance the OpenCV CPU
adapter's fixtures compare against, and behind the profile recorded in
[ADR 0003](../../adr/0003-opencv-matching-profile-and-public-score.md).

Unlike the `G-014` archive evidence, no probe was written and thrown away. The
reports come from a tracked example that searches the same fixtures the test suite
searches, so a report cannot drift away from what is verified on every pull
request:

```sh
cargo run --release --locked --package mado-pilot-backend-opencv \
    --example match-report -- --label "<host, OS build, OpenCV, compiler>"
```

The example detects nothing about the host beyond architecture and operating-system
family. CPU model, operating-system build, OpenCV build, and compiler are stated in
the label, because a program that guesses them records a guess.

| File | Contents |
|---|---|
| `report-aarch64-apple-darwin.json` | Apple Silicon fixture report |
| `report-x86_64-pc-windows-msvc.json` | Windows 11 x64 fixture report |

Only the reports are tracked. The full test-run logs from each host are not: they
are hundreds of lines of unrelated test names that go stale the moment a test is
added anywhere in the workspace, and what they establish is stated below instead.
Both hosts ran `cargo test --locked --workspace --all-targets` green, including the
shared vision contract suite and all twelve algorithm fixtures. Their totals differ
— 432 on macOS against 427 on Windows — entirely because five asset-loading tests
are `#[cfg(unix)]`; no vision test is gated on either target.

## Hosts

| Field | Apple Silicon | Windows |
|---|---|---|
| Release target | `aarch64-apple-darwin` | `x86_64-pc-windows-msvc` |
| CPU | Apple M1 Pro, 10 logical cores | Intel Core i7-12700KF |
| Operating system | macOS 26.5.2 (build 25F84) | Windows 11 Pro 25H2 |
| Toolchain | rustc 1.97.1 (8bab26f4f 2026-07-14) | rustc 1.97.1 |
| OpenCV | 4.14.0, Homebrew `opencv@4`, shared libraries | 4.14.0, official prebuilt, `opencv_world4140` |
| libclang | Xcode Command Line Tools, Apple clang 21.0.0 | LLVM 22.1.8 |
| Build profile | `release`, default settings | `release`, default settings |

Both hosts run OpenCV 4.14.0, so a score difference between them is attributable
to the CPU, the compiler, and the OpenCV build, and not to an algorithm change
between versions.

## The fixtures

All seven come from `mado-pilot-testkit`'s `match_fixtures` module, which builds a
96×64 scene from integer arithmetic on the pixel coordinate — identical bytes on
both targets, which is what makes a score comparison a statement about the
backend. The scene is pseudo-random noise carrying three copies of one 12×10
patch: two exact copies at `(20, 12)` and `(60, 40)`, and one half-strength copy
at `(20, 44)` blended with the noise under it.

| Fixture | What it establishes |
|---|---|
| `planted-full-frame-rgba` | Both exact copies found at their planted offsets, one match each |
| `planted-full-frame-bgra` | The mapping's channel swap does not move a result |
| `planted-with-degraded-copy` | A weaker copy is admitted at a lower threshold and ordered last |
| `planted-region-of-interest` | A region search reports full-frame coordinates |
| `planted-clipped-region` | A clipped region reports full-frame coordinates |
| `absent-best-offset` | The noise floor a "no match" fixture's threshold sits above |
| `oversized-template` | A template larger than the frame is a successful empty result |

## What the two runs measured

Every match rectangle, result count, template identity, searched region, and
ordering is **identical** on the two targets. The scores below name the target only
where they differ.

| Fixture | Matches | Scores |
|---|---|---|
| `planted-full-frame-rgba` | `[20, 12, 32, 22]`, `[60, 40, 72, 50]` | `1.0`, `1.0` |
| `planted-full-frame-bgra` | identical | identical |
| `planted-with-degraded-copy` | the two above, then `[20, 44, 32, 54]` | `1.0`, `1.0`, then `0.756341159343719` (macOS) / `0.756341457366943` (Windows) |
| `planted-region-of-interest` | `[60, 40, 72, 50]`, searched `[55, 35, 80, 60]` | `1.0` |
| `planted-clipped-region` | `[60, 40, 72, 50]`, searched `[55, 35, 96, 64]` | `1.0` |
| `absent-best-offset` | eight offsets, best `[55, 7, 67, 17]` | `0.127169698476791` down to `0.101423330605030`, bit-identical on both |
| `oversized-template` | none | — |

The absent patch's best correlation anywhere in the scene is `0.127`, so the
`0.9` threshold the "no match" fixtures use has a margin of more than seven tenths
under it. The fixtures assert a `0.3` ceiling, which is where that margin would
stop being evidence rather than where it currently sits.

## Why the tolerance exists

The tolerance is not a concession to the two hosts disagreeing — they barely
disagree. It is a property of the algorithm, and the measurement that shows it came
from a single host.

An earlier revision of the scene planted only the two exact copies. Both windows
held byte-identical pixels, so both correlations are mathematically one — and the
measured scores were `1.0` and `1.0 - 3.6e-7`. Adding the third, degraded copy
elsewhere in the same scene moved both to exactly `1.0`.

That is `matchTemplate` doing what it documents: it normalizes through integral
images over the whole image and computes the correlation numerator for all offsets
at once rather than window by window, so every score carries rounding from
arithmetic that involved pixels outside its own window. Two identical windows in
one image are therefore not guaranteed identical scores, and a score is not stable
under an unrelated change elsewhere in the scene.

Three consequences are built into the fixtures:

- An exactly aligned copy's score is compared against `1.0` with a `1e-5`
  tolerance, more than an order of magnitude above the `3.6e-7` variation observed
  within one host. Exact comparison would be asserting rounding.
- The degraded copy's score is compared with a wider `1e-3` band, because `1.0` is
  a definition while `0.756341` is a measurement of one OpenCV build. CI installs
  `opencv@4` from Homebrew and the official 4.14.0 prebuilt on Windows, so a patch
  release can move that digit without anything being wrong. The band is still far
  narrower than any profile change would produce.
- No fixture asserts an ordering between two candidates whose scores differ by
  less than the tolerance. The two exact copies are compared as a set; ordering is
  asserted only against the degraded copy, which sits `0.24` below them.

Both CI jobs print a full report at the end of their run, so the OpenCV version and
the scores a given run actually produced are in the log. A provisioning drift is
therefore visible directly rather than only as a failing tolerance.

## Reconciliation

Both runs are recorded. They were compared in this order, because a disagreement in
any of the first five is a defect rather than a tolerance to widen:

| Compared | Result |
|---|---|
| Public outcome and error category | Identical for all seven fixtures |
| Result count | Identical: 2, 2, 3, 1, 1, 8, 0 |
| Template identities | Identical |
| Capture-pixel bounds and searched regions | Identical, every rectangle |
| Canonical ordering | Identical in every fixture |
| Public scores | One difference, `2.98e-7` |

**The only numeric difference between the two targets, anywhere in the reports, is
the degraded copy's score: `2.98e-7`, which is exactly five units in the last place
of the `f32` the backend computed.** Sixteen of the seventeen reported scores are
bit-identical, including all eight of the absent template's noise-floor offsets.

That is a stronger result than the tolerance was written to accommodate, and it
does not change the tolerance. The `1e-5` band stays, for two reasons. It leaves
34× margin over the largest difference measured, and — more to the point — the
variation it exists for is not a cross-target effect at all: the `3.6e-7` shift
documented below was observed *within one host*, from changing an unrelated part of
the same scene. A band tightened to the cross-target agreement would be a band that
one host can violate on its own.

Reproducing the comparison needs no stored log: the reports are what the tracked
example prints, and the example searches the fixtures the test suite searches.
