# Validation gates

A validation gate is a version-one decision that is deliberately unresolved
because the evidence needed to settle it does not exist yet. Each gate records
what is undecided, what evidence resolves it, when it must be resolved, what it
blocks, and how the resolution is recorded.

Gates exist so that an unresolved decision stays visible instead of being settled
by accident in an implementation pull request. A gate is not permission to weaken
an architectural seam recorded in [architecture.md](architecture.md).

## How a gate is used

Before entering or exiting a phase, check every gate whose **Due** column names
that phase. A phase must not exit while a gate that blocks its exit is open, and
a phase must not begin implementing behavior that an open gate blocks.

Resolving a gate requires all of the following in one change:

1. The evidence named in the gate, committed or referenced from a tracked
   location.
2. An architecture decision record created from
   [adr/0000-template.md](adr/0000-template.md), recording the decision, the
   evidence, the rejected alternatives, and the consequences.
3. A synchronized update to [architecture.md](architecture.md) and any affected
   specification, test, policy, or example.
4. An update to this registry marking the gate resolved with its ADR number, or
   removing the gate when the underlying claim was withdrawn instead.

A gate is never presented as resolved on the strength of a plan, an opinion, or a
successful build alone.

## Status of Phase 0

No gate blocks Phase 0. Phase 0 is nonetheless incomplete until every gate below
has an explicit due phase, blocking scope, and resolution rule, because that
registry is itself a Phase 0 deliverable.

## Registry

| ID | Unresolved decision | Due | Blocks | Status |
|---|---|---|---|---|
| [`G-001`](#g-001) | Minimum Windows and macOS versions | Before Phase 2 exit | Native support claim and release | Open |
| [`G-002`](#g-002) | Windows capture producer-pool and frame-detachment strategy | Before Phase 2 implementation | Windows capture ownership | Open |
| [`G-003`](#g-003) | macOS shim language | Before Phase 2 implementation | macOS shim implementation | Open |
| [`G-004`](#g-004) | Default OCR model profile | Before Phase 3 implementation | Default OCR profile | Open |
| [`G-005`](#g-005) | Default change-detection algorithm and threshold | Before Phase 4 implementation | Default watcher policy | Open |
| [`G-006`](#g-006) | Acceleration candidates and provider ordering | Before Phase 5 implementation | Acceleration defaults | Open |
| [`G-007`](#g-007) | Native dependency bundling profiles | Before Phase 5 implementation | Release packaging | Open |
| [`G-008`](#g-008) | Static-library feasibility | Before Phase 5 exit | Static artifact claim only | Open |
| [`G-009`](#g-009) | Stable public Rust item names | Before Phase 1 exit | Rust stability promise | Open |
| [`G-010`](#g-010) | Version-one C ABI status, prefix, and layout | Before Phase 1 exit | ABI compatibility baseline | Open |
| [`G-011`](#g-011) | Native-frame extension discovery | Future roadmap | Does not block version one | Deferred |
| [`G-012`](#g-012) | Published Cargo and C build profiles | Before Phase 5 implementation | Release capability matrix | Open |
| [`G-013`](#g-013) | Numeric benchmark budgets | Before each affected phase exits | That phase's exit | Open per workload |
| [`G-014`](#g-014) | Archive safety ceilings | Before Phase 1 implementation | Version-one archive loading | Resolved by [ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md) |

## G-001

**Unresolved decision.** The exact minimum supported Windows and macOS versions
that published artifacts declare.

**Required evidence.** Build and load probes on the candidate oldest systems for
both release targets, including the behavior of every capability that uses an API
newer than the declared minimum.

**Due.** Before Phase 2 exit.

**Blocks.** Any native support claim, and release.

**Status.** Open.

**Resolution.** An ADR that records the chosen minimums, the probe results, and
the availability-check or weak-linking strategy for newer APIs, followed by
synchronized platform support documentation.

## G-002

**Unresolved decision.** The Windows Graphics Capture producer-pool sizing,
public-frame detachment, and texture-reuse strategy.

**Required evidence.** A retained-frame stress prototype showing that published
frames do not pin producer buffer-pool slots that capture progress requires, and
that native resources outlive in-flight mapping and backend work.

**Due.** Before Phase 2 implementation.

**Blocks.** Windows capture ownership design.

**Status.** Open.

**Resolution.** An ADR recording the prototype measurements and the chosen
ownership rule, followed by the retained-frame and producer-progress contract
tests that enforce it.

## G-003

**Unresolved decision.** Whether the macOS capture and input shim is written in
Objective-C or Objective-C++.

**Required evidence.** A prototype covering exception behavior across the language
boundary, object ownership, and build integration on Apple Silicon.

**Due.** Before Phase 2 implementation.

**Blocks.** macOS shim implementation.

**Status.** Open.

**Resolution.** An ADR recording the prototype outcome and the containment rule
for exceptions crossing the shim boundary.

## G-004

**Unresolved decision.** The default OCR model, its language set, size,
preprocessing metadata, expected hash, and license.

**Required evidence.** A cross-target quality fixture showing reproducible
recognition results on both release targets, plus a license review confirming
redistribution is permitted.

**Due.** Before Phase 3 implementation.

**Blocks.** The default OCR profile.

**Status.** Open.

**Resolution.** An ADR recording the model choice, the fixture results, and the
license and deployment obligations, followed by an update to
[third-party-dependencies.md](third-party-dependencies.md).

## G-005

**Unresolved decision.** The default change-detection algorithm and its threshold
for watcher scheduling.

**Required evidence.** A false-skip evaluation over recorded frame sequences,
showing how often a real change would be skipped at the chosen threshold.

**Due.** Before Phase 4 implementation.

**Blocks.** The default watcher policy.

**Status.** Open.

**Resolution.** An ADR recording the evaluation and the chosen default, plus the
recorded sequences kept as regression fixtures.

## G-006

**Unresolved decision.** The Core ML and Windows acceleration candidates and the
execution-provider ordering.

**Required evidence.** Compatibility and correctness runs for each candidate
provider on its release target, including the observable behavior when a provider
is rejected during model loading.

**Due.** Before Phase 5 implementation.

**Blocks.** Acceleration defaults.

**Status.** Open.

**Resolution.** An ADR recording the candidate results and the provider ordering,
plus the fallback policy that limits fallback to model loading.

## G-007

**Unresolved decision.** Whether OpenCV and ONNX Runtime are bundled or consumed
from a controlled host-provided installation, and which release profiles exist.

**Required evidence.** Clean-system package prototypes for each candidate profile
on both release targets, plus a license review of every redistributed artifact.

**Due.** Before Phase 5 implementation.

**Blocks.** Release packaging.

**Status.** Open.

**Phase 1 input.** Phase 1 links OpenCV 4.14.0 as a *development prerequisite* and
claims nothing about a release. Two facts it established belong to this gate. The
library is Apache-2.0, the same licence as this project, so bundling it would add
an attribution obligation and no new term. And because OpenCV is linked dynamically
at load time, an absent library stops the process before any MadoPilot code runs,
so it cannot be reported as an actionable status — the adapter reports an
unsupported *version* and nothing more. Closing that gap is part of this gate's
controlled library search paths; it is not `G-008`, whose scope is static-link
feasibility. See
[third-party-dependencies.md](third-party-dependencies.md#opencv).

**Resolution.** An ADR recording the profile matrix, the controlled library search
paths, and the license and notice obligations, followed by updates to
[third-party-dependencies.md](third-party-dependencies.md) and the packaging
documentation.

## G-008

**Unresolved decision.** Whether a static library is feasible for each advertised
dependency combination.

**Required evidence.** Link results and license review for each combination that a
static artifact would advertise.

**Due.** Before Phase 5 exit.

**Blocks.** The static artifact claim only. It does not block the shared-library
release.

**Status.** Open.

**Resolution.** An ADR that either records the supported static combinations with
their evidence, or withdraws the static artifact claim.

## G-009

**Unresolved decision.** The stable public Rust item names of the facade.

**Required evidence.** The Phase 1 vertical slice with working Rust examples, and
an interface review of the names those examples exercise.

**Due.** Before Phase 1 exit.

**Blocks.** Any Rust API stability promise.

**Status.** Open. Phase 0 deliberately reserves package and artifact names only;
see the naming baseline in [architecture.md](architecture.md).

**Resolution.** An ADR recording the reviewed names and the compatibility policy
that then applies to them.

## G-010

**Unresolved decision.** The exact version-one C status codes, the mandatory
function-table prefix, the structure layouts, and the mapping from Rust errors to
C status values.

**Required evidence.** The Phase 1 minimal C ABI exercised by C and C++ examples,
with owned-handle lifecycle, structure-size negotiation, and error-ownership tests
passing.

**Due.** Before Phase 1 exit.

**Blocks.** The ABI compatibility baseline, and therefore every later
old-header-prefix compatibility claim.

**Status.** Open. This gate is deliberately separate from `G-011`.

**Resolution.** An ADR freezing the allocation and layout rules, followed by the
ABI layout and old-header compatibility tests that enforce them.

## G-011

**Unresolved decision.** How a native-frame extension table is discovered and
allocated in the C ABI.

**Required evidence.** A portable extension prototype covering discovery,
allocation, and version negotiation.

**Due.** Future roadmap.

**Blocks.** Nothing in version one.

**Status.** Deferred. An ADR is required if this is activated.

**Resolution.** An ADR recording the prototype and the negotiation rule, plus a
scope update that moves the feature out of the future roadmap.

## G-012

**Unresolved decision.** The published Cargo feature defaults and the C build
profiles.

**Required evidence.** A feature-matrix build across both release targets with
binary-size measurements for each profile.

**Due.** Before Phase 5 implementation.

**Blocks.** The release capability matrix.

**Status.** Open.

**Resolution.** An ADR recording the matrix and the chosen defaults, plus a
build-profile capability table published with the release.

## G-013

**Unresolved decision.** The numeric benchmark budgets for every workload a phase
introduces.

**Required evidence.** Repeatable baseline measurements captured with the profile
format in [performance.md](performance.md), on the release target the workload
runs on.

**Due.** Before each affected phase exits.

**Blocks.** That phase's exit.

**Status.** Open per workload. Phase 0 defines the profile and budget format and
deliberately assigns no numeric product budget.

**Resolution.** Committed benchmark profiles and budgets plus an ADR for each
budget that is set or relaxed, recording the evidence behind the number.

## G-014

**Decision.** The archive entry-count, uncompressed-byte, and compression-ratio
safety ceilings for asset loading.

**Required evidence.** Adversarial fixtures covering traversal, links, special
files, duplicate normalized entries, and decompression bombs, each rejected
deterministically at the chosen ceilings.

**Due.** Before Phase 1 implementation.

**Blocks.** Version-one archive loading.

**Status.** Resolved by
[ADR 0001](adr/0001-asset-archive-container-and-safety-ceilings.md). The
container is ZIP restricted to `Stored` and `Deflated`, the manifest is strict
UTF-8 JSON, and six implementation ceilings are set: 4 MiB of manifest bytes,
4,096 entries, 64 MiB per entry, 256 MiB of source bytes, 512 MiB of total
uncompressed bytes, and an expansion ratio of 64. A caller may lower a limit and
may not raise one. Three ceilings beyond the ones this gate named were added,
because entry count, expansion bytes, and ratio do not on their own bound what a
loader allocates.

The evidence is in [evidence/g-014](evidence/g-014/) and the fixtures in
[../fixtures/assets/g-014](../fixtures/assets/g-014/). `mado-pilot-assets`
implements the decision, and its conformance suite asserts the failure category
and the refusing stage for every tracked adversarial fixture, on both release
targets. A fixture refused later than its documented stage fails that suite even
though the package was refused, which is what keeps an earlier guard from being
quietly removed.

**Resolution.** An ADR recording the ceilings and the adversarial fixture results,
followed by the asset schema and security documentation that states them.
