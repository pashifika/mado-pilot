# Phase 1 deterministic-slice asset package

A minimal valid asset package for the Phase 1 Rust workflow. It exists so the
example, the facade tests, and the benchmark harness all load the same package
from disk instead of each building one, and so the repository has one worked
manifest that is not gate evidence.

It is not `G-014` evidence. The adversarial and representative fixtures that
resolved that gate are in [../g-014](../g-014/) and are pinned by their own
`SHA256SUMS`; this package is pinned by the digests inside its own manifest,
which the loader verifies on every load.

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
regeneration is visible immediately.
