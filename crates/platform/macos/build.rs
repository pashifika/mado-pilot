//! Compiles the macOS native shim and declares its framework linkage.
//!
//! This build script owns the shim's compilation flags and framework linkage
//! rather than inheriting them from a binding crate, which is rule 8 of
//! `docs/adr/0012-macos-shim-language-and-containment.md`.
//!
//! Two of the flags are correctness requirements rather than preferences.
//! `-fobjc-arc-exceptions` is the one the ADR measured: without it, ARC emits no
//! release on an exception's unwind edge, so an exception raised where a failing
//! stream start would raise one leaks the native object the session had already
//! retained. `MP_SHIM_ARC_EXCEPTIONS` is defined beside it because Clang exposes
//! no feature macro for the flag, so the shim's `#error` and the flag stay in one
//! review.
//!
//! The frameworks declared below all predate every macOS version this project
//! could select as its minimum, so linking them is safe on any supported host.
//! ScreenCaptureKit is deliberately absent: the shim loads it from its absolute
//! system location at runtime, because an eager dependency on a framework that
//! arrived in 12.3 would make an older host fail to load instead of reporting an
//! unsupported status.

fn main() {
    println!("cargo::rerun-if-changed=native/madopilot_macos_shim.m");
    println!("cargo::rerun-if-changed=native/madopilot_macos_shim.h");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        // The package keeps a documented empty seam off macOS and resolves no
        // macOS dependency, so there is nothing to compile.
        return;
    }

    cc::Build::new()
        .file("native/madopilot_macos_shim.m")
        .include("native")
        .flag("-fobjc-arc")
        .flag("-fobjc-arc-exceptions")
        .define("MP_SHIM_ARC_EXCEPTIONS", "1")
        .warnings(true)
        .extra_warnings(true)
        .compile("madopilot_macos_shim");

    for framework in [
        "ApplicationServices",
        "CoreFoundation",
        "CoreGraphics",
        "CoreMedia",
        "CoreVideo",
        "Foundation",
    ] {
        println!("cargo::rustc-link-lib=framework={framework}");
    }
}
