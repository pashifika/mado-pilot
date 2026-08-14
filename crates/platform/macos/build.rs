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
//! The workspace declares macOS 26.5.2 as this implementation's deployment floor.
//! ScreenCaptureKit remains deliberately absent from eager linkage: controlled
//! loading keeps capability failure typed and lets the linkage test prove the shim
//! has no ambient-framework dependency.
//!
//! `MADO_PILOT_MACOS_ASAN=1` additionally builds the shim under AddressSanitizer.
//! That mode exists because the session's ownership scenarios assert that a live
//! native object *count* returns to its baseline, and a count cannot observe an
//! access after a free — so a use-after-free in the shim passes them.
//!
//! `CONTRIBUTING.md` step 10 is the command, and records why each part of it is
//! load-bearing. What belongs here is the reason it is opt-in: the sanitizer runtime
//! is a process-wide dependency that a released artifact must not carry, and it is
//! linked into this package's own test binaries alone, so a build that consumed the
//! instrumented shim from elsewhere would fail to resolve its symbols.
//!
//! Running it needs the capture scenarios to actually capture, which needs Screen
//! Recording granted to the test process; `mado_pilot_asan` is published below so
//! that a scenario which cannot reach a capture fails this build instead of skipping.
const NATIVE_FRAMEWORKS: [&str; 6] = [
    "ApplicationServices",
    "CoreFoundation",
    "CoreGraphics",
    "CoreMedia",
    "CoreVideo",
    "Foundation",
];

/// Reads whether this build was asked for an AddressSanitizer-instrumented shim.
///
/// The answer is also published as `mado_pilot_asan`, because a scenario that
/// cannot reach a live capture has to fail this build rather than skip: the
/// sanitizer observes a freed access performed *during* a capture, so a run that
/// captured nothing reports nothing and would otherwise be indistinguishable from
/// a clean one.
fn address_sanitizer_requested() -> bool {
    println!("cargo::rerun-if-env-changed=MADO_PILOT_MACOS_ASAN");
    println!("cargo::rustc-check-cfg=cfg(mado_pilot_asan)");
    let requested = std::env::var("MADO_PILOT_MACOS_ASAN").is_ok_and(|value| value == "1");
    if requested {
        println!("cargo::rustc-cfg=mado_pilot_asan");
    }
    requested
}

/// Links the sanitizer runtime that the instrumented shim's references need.
///
/// Passing `-fsanitize=address` to the link is not enough. `rustc` invokes the
/// linker driver with `-nodefaultlibs`, under which the driver adds no sanitizer
/// runtime, and the shim's `__asan_*` references are then reported as undefined
/// symbols. The runtime is therefore named explicitly, and it is taken from the
/// resource directory of the compiler that instrumented the shim so that the two
/// cannot disagree about its version. Its install name is `@rpath`-relative, so
/// the directory is also added as a run-path.
fn link_address_sanitizer_runtime(shim: &cc::Build) {
    let compiler = shim
        .try_get_compiler()
        .expect("the shim's compiler is resolvable on macOS");
    let reported = std::process::Command::new(compiler.path())
        .arg("-print-resource-dir")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "MADO_PILOT_MACOS_ASAN needs the resource directory of {}, which \
                 could not be read: {error}",
                compiler.path().display()
            )
        });
    assert!(
        reported.status.success(),
        "{} reported no resource directory, so the AddressSanitizer runtime cannot \
         be located",
        compiler.path().display()
    );
    let resource =
        String::from_utf8(reported.stdout).expect("a compiler resource directory is valid UTF-8");
    let directory = std::path::Path::new(resource.trim()).join("lib/darwin");
    let runtime = directory.join("libclang_rt.asan_osx_dynamic.dylib");
    assert!(
        runtime.is_file(),
        "MADO_PILOT_MACOS_ASAN needs {}, which this toolchain does not provide. The \
         Xcode Command Line Tools ship it; a compiler selected through CC may not.",
        runtime.display()
    );

    println!("cargo::rustc-link-arg={}", runtime.display());
    println!("cargo::rustc-link-arg=-Wl,-rpath,{}", directory.display());
}

/// Compiles and links the interactive fixture's window only for its private bin.
///
/// `cc` metadata is disabled so no `-l madopilot_macos_input_fixture` directive
/// can reach the production library or any consumer. The resulting archive is
/// passed as a target-specific linker argument to the feature-gated fixture
/// executable. This is explicit isolation rather than archive dead stripping.
///
/// The fixture reuses the production execution-context classifier and therefore
/// links the production shim plus its foundational frameworks into this private
/// executable only. AppKit and the opt-in OpenGL renderer remain controlled
/// runtime loads rather than eager dependencies.
fn compile_input_fixture() {
    println!("cargo::rerun-if-changed=native/madopilot_macos_input_fixture.m");
    println!("cargo::rerun-if-changed=native/madopilot_macos_input_fixture.h");

    cc::Build::new()
        .file("native/madopilot_macos_input_fixture.m")
        .include("native")
        .flag("-fobjc-arc")
        .flag("-fobjc-arc-exceptions")
        .flag("-mmacosx-version-min=26.5.2")
        .warnings(true)
        .cargo_metadata(false)
        .extra_warnings(true)
        .compile("madopilot_macos_input_fixture");
    let archive = std::path::PathBuf::from(
        std::env::var_os("OUT_DIR").expect("Cargo provides an output directory"),
    )
    .join("libmadopilot_macos_input_fixture.a");
    assert!(
        archive.is_file(),
        "the private fixture archive was not produced at {}",
        archive.display()
    );
    println!(
        "cargo::rustc-link-arg-bin=mado-pilot-macos-input-fixture={}",
        archive.display()
    );
    let production_archive = archive.with_file_name("libmadopilot_macos_shim.a");
    assert!(
        production_archive.is_file(),
        "the production shim archive was not produced at {}",
        production_archive.display()
    );
    println!(
        "cargo::rustc-link-arg-bin=mado-pilot-macos-input-fixture={}",
        production_archive.display()
    );
    for framework in NATIVE_FRAMEWORKS {
        println!(
            "cargo::rustc-link-arg-bin=mado-pilot-macos-input-fixture=-Wl,-framework,{framework}"
        );
    }
}

fn main() {
    println!("cargo::rerun-if-changed=native/madopilot_macos_shim.m");
    println!("cargo::rerun-if-changed=native/madopilot_macos_shim.h");

    let sanitize = address_sanitizer_requested();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        // The package keeps a documented empty seam off macOS and resolves no
        // macOS dependency, so there is nothing to compile.
        return;
    }

    let mut shim = cc::Build::new();
    shim.file("native/madopilot_macos_shim.m")
        .include("native")
        .flag("-fobjc-arc")
        .flag("-fobjc-arc-exceptions")
        .define("MP_SHIM_ARC_EXCEPTIONS", "1")
        .flag("-mmacosx-version-min=26.5.2")
        .warnings(true)
        .extra_warnings(true);

    if sanitize {
        // Instrumenting the shim is what makes the freed access observable, and
        // linking the runtime into this package's test binaries is what makes the
        // instrumentation resolvable. The frame pointer is kept so the report
        // names the shim function that performed the access rather than an
        // address inside it.
        shim.flag("-fsanitize=address")
            .flag("-fno-omit-frame-pointer");
        link_address_sanitizer_runtime(&shim);
    }

    shim.compile("madopilot_macos_shim");
    if std::env::var_os("CARGO_FEATURE_PRIVATE_FIXTURE").is_some() {
        compile_input_fixture();
    }

    for framework in NATIVE_FRAMEWORKS {
        println!("cargo::rustc-link-lib=framework={framework}");
    }
}
