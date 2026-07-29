# C ABI: cross-language layout evidence

This directory holds what the C compiler and `rustc` each measured for the Phase 1
C structures, on each release target. It was the layout half of the evidence
[`G-010`](../../validation-gates.md#g-010) needed, and is now the record of what
that gate froze.

The header at `crates/bindings/capi/include/madopilot/madopilot.h` is
hand-written rather than generated;
[ADR 0004](../../adr/0004-c-header-authorship-and-abi-verification.md) records why,
and the consequence is that the header's agreement with the Rust `#[repr(C)]`
definitions has to be proved rather than assumed. It is proved by comparison
rather than by assertion: the same list of structures is measured twice, once by
each compiler, and the two reports must be identical line for line.

```sh
cargo build --locked --package mado-pilot-capi
cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
```

The report files here are the agreed output of that comparison. A file is
therefore evidence of two things at once: what the layout *is* on that target, and
that both compilers produced it.

That command now also compiles and runs the C++ ownership probe, the C++ example,
and the CMake consumer project. Those produce no layout numbers — the C++ wrapper
declares no ABI of its own — so nothing about them is recorded here. What they
contributed to `G-010` is behavioural, and is recorded in the gate's evidence
table rather than as a file.

**What is recorded here is frozen.**
[ADR 0007](../../adr/0007-phase-1-c-abi-freeze.md) froze these layouts as ABI
major 1, and the same change copied the header's declarations into the permanent
old-prefix fixture at
[`tests/abi-compat/v1`](../../../crates/bindings/capi/tests/abi-compat/v1/),
where they are not edited again. The working header has gained comments since,
so the two files no longer match byte for byte; nothing compares them, and what
the fixture pins is the declarations these reports measured. A report that
changes from here is either an additive minor — a new structure, or a field
appended after every existing one — or a defect.

## Hosts

| Target | File | Host |
|---|---|---|
| `aarch64-apple-darwin` | [layout-aarch64-apple-darwin.txt](layout-aarch64-apple-darwin.txt) | macOS 26.5.2, Apple clang 21.0.0, rustc 1.97.1, OpenCV 4.14.0, CMake 4.4.0 |
| `x86_64-pc-windows-msvc` | [layout-x86_64-pc-windows-msvc.txt](layout-x86_64-pc-windows-msvc.txt) | Windows 11 Pro 10.0.26200, MSVC 19.37.32824, rustc 1.97.1, OpenCV 4.14.0, CMake 3.29.5 |

Those rows are no longer transcribed by hand. `c-abi-check` prints the version
of every tool it uses — both compilers, CMake, and the `rustc` on the other side
of the comparison — so a run's own output states the conditions it was measured
under. It used to discover them and throw them away.

The C++ compiler is the same driver as the C one on both hosts, and the CMake
versions are recorded because stage 7 made CMake a prerequisite of the check
that produced these reports. Neither report changed when it did: the C++ wrapper
is header-only and declares no ABI, so it adds nothing to lay out. The two
versions above are also the widest spread the check has been run against — a
CMake 3 and a CMake 4 — against a declared minimum of 3.22.

**The two reports are byte-identical.** That is worth stating because it was not
required: the C ABI is per-platform, and two targets are free to lay the same
declarations out differently. They did not, which means the structure rule —
`struct_size` followed by a 32-bit field, no implicit padding, views and
rectangles as fixed primitives — produces one layout on both release targets
rather than two that each happen to work. A future target that disagrees is
therefore a fact about that target, not a defect in the rule.

CI runs the same comparison on every pull request, on `macos-15` and
`windows-2025`, so a divergence is caught there as well as on a verification host.
What a tracked report adds is the exact numbers, on a named host, at a point in
time — which is what an ADR has to cite.

## Reading a report

Three line shapes, in the order the structures are declared:

```text
type <name> size=<bytes> align=<bytes>
field <name>.<field> offset=<bytes>
handle <name> size=<bytes> align=<bytes>
```

A `handle` line is an opaque pointer. Its only interesting property is that it is
thin: an opaque type a C caller cannot size must not become a fat pointer on the
Rust side.

The `madopilot_api_t` entry is the function table, and its field offsets are the
Phase 1 member order. That order is the part of this file with the longest
consequences: within an ABI major, members are only ever appended.
