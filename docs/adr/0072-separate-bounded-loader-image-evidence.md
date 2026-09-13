# ADR 0072: Separate bounded loader image evidence from program output

- **Status:** Accepted
- **Date:** 2026-09-13
- **Resolves gate:** none; real replay and numerical qualification remain open
- **Supersedes:** none

## Context

The first authorized Apple binding on source `3dcae30` stopped at its fixed
64-KiB combined-output limit. All533 retained lines were dyld image diagnostics;
no semantic result was observed. Owned cleanup succeeded. That attempt remains
a failure; neither its limit nor its evidence is changed.

The manifest contract already distinguishes at most256 non-system Apple images
from OS shared-cache images. Counting every system image before applying that
scope can also reject an otherwise bounded non-system manifest. Increasing or
silently filtering the ordinary output budget would not repair either mismatch.

## Decision

Use dyld's documented `DYLD_PRINT_TO_FILE` destination for a separate private
image report. A Darwin-only launcher creates the report exclusively with mode
0600, sets a kernel `RLIMIT_FSIZE` ceiling of at most1MiB without relaxing an
inherited lower limit, restores default `SIGXFSZ`, then exec-replaces itself
with the exact candidate. `DYLD_PRINT_*` is set only for that exec, so launcher
startup images are not mixed into candidate image evidence. Ambient dyld print
settings are refused.

The existing process supervisor still enforces the unchanged ordinary stdout/
stderr and time/cleanup limits. It supervises the same PID through exec; process
launch success is not proof that candidate main or OCR ran. Its whole-process
wall time and process peak can include launcher startup. The file-size ceiling
applies to all candidate regular-file writes, not just the report; this is part
of the new execution applicability and must be accepted before a fresh run.

Retain and hash the image report separately. Missing, partial, saturated or
malformed required metadata is nonpass. Exclude OS shared-cache paths before
applying the256-image Apple manifest bound. Windows continues using its existing
bounded module report. The source, interpreter and launcher are bound alongside
the candidate; no generic supervisor or public runtime API changes.

## Verification and limits

Model-free owned Python children demonstrate that ordinary stdout/stderr remain
unchanged while dyld diagnostics go to the private file. A real over-limit write
is stopped at the file ceiling and cannot become dependency proof. Synthetic
reports verify that600 system images do not consume the non-system manifest
capacity and that saturated/partial reports are rejected. No failed model
candidate is rerun by these tests.

This correction does not approve a native image set, a repeated binding, new
model/capture execution, a numerical ceiling, or Windows asset acquisition.
The first Apple output-limit failure and Windows archive-expansion refusal
remain historical failures. Windows prerequisites remain operator-owned under
`D:\oss_libs`, with no assistant download, transfer or extraction.
