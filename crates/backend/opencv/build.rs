//! Compiles the optional benchmark thread-observation boundary.

fn main() {
    println!("cargo::rerun-if-changed=src/benchmark_threads.cpp");
    #[cfg(feature = "benchmark-selection")]
    {
        let mut build = cc::Build::new();
        build
            .cpp(true)
            .std("c++17")
            .file("src/benchmark_threads.cpp");
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            build.flag("/EHsc");
        } else {
            build.flag("-fexceptions");
        }
        build.compile("madopilot_benchmark_threads");
    }
}
