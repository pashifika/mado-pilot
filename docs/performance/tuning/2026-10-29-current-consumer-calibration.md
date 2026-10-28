# Current Apple consumer calibration

## Decision and scope

Retain the complete `group-roi` baseline calibration as a numerical non-pass.
All six invocations pass their application, runtime-identity and owned-cleanup
contracts. The fixed repeatability condition fails; no unchanged A/A control,
profile or candidate comparison follows. No implementation change, threshold
change, retry, performance regression or release approval is inferred.

The [allowlisted result](../../evidence/p050-current-consumer-calibration/apple-group-roi.json)
retains all six scalar rows, the original issue and immutable evidence hashes.
This is a new qualification attempt, separate from the earlier
[group observations](2026-10-26-actual-consumer-group-observation.md) and
[candidate-only applicability](2026-10-27-current-consumer-source-applicability.md).
Their results remain unchanged.

## Source and measurement boundaries

The original frozen consumer engines are reused, without Cargo or rebuilding:
baseline executable `3e8f29fba8350a76cca491c6fffc04d509afdee2c52bd223d0f61308d2448bda`
and the unexecuted candidate executable
`4df96a0d31ed337ce46065c2f517ce58ad4c19461f8d01e903a9d95ec30bebab`.
Their original source graphs and compiled identities remain distinct from the
current collector. Both are Rust 1.98.1 dev-normal artifacts: opt-level 0,
debuginfo 2, debug assertions and observations enabled, counting allocator off.
This is not an optimized-release or observation-off result.

The current method binds the actual loader-on runtime closure, source-owned
process contracts, measured observer work, finite retention and explicit
resident policies. CPU remains diagnostic. The existing Campaign is reused;
no root reset or fence removal admits the attempt.

Seventeen wall-clock metrics are graded: the group span and eight recognize /
eight release intervals. The group span runs from the first recognize start to
the eighth release end in one child-local epoch, including intervening host and
JavaScript work. It excludes process startup, preflight, later assertions and
shutdown. Two separate sampled-RSS maxima have a fixed 268,435,456-byte ceiling
and a 1,048,576-byte comparison floor; they are not OS peak, same-owner growth,
allocator accounting or leak measurements.

## Direct clock evidence

Before calibration, a separately compiled finite probe observes 8 batches of
1,024 back-to-back `Instant::now()` pairs on the current host. Both timestamps
use the same epoch and integer-nanosecond conversion as the frozen callers.
All raw pairs, including two zero differences and the maximum observed gap of
19,958 ns, are retained.

The predeclared rule is the maximum of the eight minimum positive differences,
plus a 1 ns reporting margin. All eight minima are 41 ns, giving 42 ns effective
resolution and a 1 ns reporting quantum. Actual compiler, standard-library
inputs, binary, link map, host and cleanup evidence are bound. Static inspection
of both frozen consumers and the probe confirms the same `CLOCK_UPTIME_RAW`
entry route. Independent review accepts this prospective applicability.

This observation includes call overhead. It is not clock accuracy, a bound on
scheduling delays, an application latency result, a dynamic call trace or proof
that complete historical standard-library archives are byte-identical. Earlier
sparse operation timestamps are not substituted for this direct observation.

## Complete calibration outcome

The stock rule uses all six observations without trimming:
`(maximum - minimum) / max(median, 2 * resolution) <= 0.15`.

| Metric | Median (ns) | Range (ns) | Range / median |
| --- | ---: | ---: | ---: |
| Group span | 65,938,479.5 | 2,671,624 | 4.051692% |
| First release | 7,000 | 5,958 | 85.114286% |

The first rejected metric in the stock method order is `release_0_ns`.
Its six values are retained in the public result. The original report records
`calibration noise exceeds 0.15`, completed execution, no unrun slots and empty
envelopes. The group-span observation does not override the release guard or
promote later unvisited envelope checks to passed. Trial-specific central-four
rules do not apply to this group calibration.

All six resource points are within their declared ceilings. Reconciled lifetime
CPU is unavailable in all six rows; original CPU diagnostics remain retained and
are not a qualification gate. Neither CPU boost, core placement nor an operating-
system cause is established by this result.

## Ownership and evidence

One public calibration CLI returns exit 1 with known cleanup, within its original
finite owner deadline. Each of the six original attempts retains its started,
raw, runtime and validation records. The parent independently reconciles the
sealed cohort and classifies a completed numerical non-pass, not an operational
failure. No successor is launched.

Public data excludes private paths, environment values, host fingerprints, PIDs
and arbitrary logs. Full originals and the accepted method remain local under
hash-bound references; this projection is not a replacement for those bytes.
The broader component goals, other consumer lanes, platform coverage and release
obligations remain unresolved where their own evidence is missing or failed.
