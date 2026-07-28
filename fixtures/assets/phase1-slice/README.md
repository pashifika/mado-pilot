# Phase 1 deterministic-slice asset package

A minimal valid asset package for the Phase 1 Rust workflow. It exists so the
example, the facade tests, and the benchmark harness all load the same package
from disk instead of each building one, and so the repository has one worked
manifest that is not gate evidence.

It is not `G-014` evidence. The adversarial and representative fixtures that
resolved that gate are in [../g-014](../g-014/) and are pinned separately.

This package is pinned twice, because the two pins cover different things. The
digests **inside** `madopilot-package.json` cover the template bytes and are
verified by the loader on every load, so a package that loads has already proved
its own contents. `SHA256SUMS` covers the manifest as well, which nothing else
can: a manifest that changed its own declared digests would still load. That
second pin is what a benchmark profile's `fixture_sha256` rests on, and
`every_tracked_slice_fixture_still_hashes_to_its_recorded_checksum` in
`crates/mado-pilot/tests/deterministic_workflow.rs` enforces both directions —
every listed file matches, and every file is listed.

## What is in it

| Template | Content | What it is for |
|---|---|---|
| `panel.patch` | The patch `mado-pilot-testkit`'s matching scene plants at `(20, 12)` and `(60, 40)` | A search that finds something |
| `panel.absent` | A patch that scene does not contain anywhere | A search that finds nothing, which is a success |

Both are 12×10 RGB PNGs authored with matching defaults of `min_score = 0.9`
and `max_results = 8`, the same thresholds the vision fixtures use, so a score
here is comparable with a score there.

## The coupling to the testkit generator

The scene these templates are searched in is generated at run time by
`mado_pilot_testkit::match_fixtures`, and `panel.patch` is only findable
because its bytes are that generator's patch. The coupling is deliberate — a
tracked 96×64 scene would be 24 KiB of pixels whose construction parameters
already say everything — and it is asserted rather than assumed: the facade
tests compare the resolved template content against the generator's output, so
a change to the generator fails a test instead of silently emptying every
result.

## Regenerating

Encode the two patches with the same generator and rewrite the digests the
manifest declares:

```rust
let width = match_fixtures::PATCH.width();
let height = match_fixtures::PATCH.height();
png::encode_rgb(width, height, &match_fixtures::patch_rgb());  // templates/patch-12x10.png
png::encode_rgb(width, height, &match_fixtures::absent_rgb()); // templates/absent-12x10.png
```

```sh
shasum -a 256 fixtures/assets/phase1-slice/templates/*.png
```

A digest that is not updated fails the load with a hash mismatch, so a stale
regeneration is visible immediately. Then regenerate `SHA256SUMS`, which covers
the rewritten manifest too:

```sh
cd fixtures/assets/phase1-slice
find . -type f ! -name SHA256SUMS ! -name README.md | sort | xargs shasum -a 256 > SHA256SUMS
```
