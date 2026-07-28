//! Compiles the C and C++ surfaces against this library and checks that they
//! agree with it and with each other.
//!
//! The header is hand-written, so its agreement with the Rust `#[repr(C)]`
//! definitions is proved rather than assumed, and the C++ wrapper is
//! header-only, so it has no artifact of its own that a build would otherwise
//! exercise. This program:
//!
//! 1. compiles and runs `tests/c/madopilot-abi-layout.c`, which reports sizes,
//!    alignments, and field offsets as the C compiler produced them;
//! 2. compares that report line by line against the same values measured from
//!    the Rust definitions;
//! 3. compiles, links, and runs `examples/c/deterministic-slice.c` against the
//!    built library and checks its outcome;
//! 4. compiles, links, and runs `tests/cpp/madopilot-cpp-ownership.cpp`, whose
//!    static assertions prove the wrapper's ownership shape at compile time and
//!    whose checks prove its behaviour at run time;
//! 5. compiles, links, and runs `examples/cpp/deterministic-slice.cpp` and
//!    checks that it answers exactly what the C example answered;
//! 6. compiles every frozen header fixture under `tests/abi-compat/` against
//!    its own header rather than the working one, links it to this library, and
//!    checks that it still negotiates and still gets the same answers;
//! 7. configures, builds, and runs the CMake consumer project in
//!    `tests/cmake/`, which reaches the library only through `MadoPilot::C` and
//!    `MadoPilot::Cpp`.
//!
//! Two compilers, one comparison. A divergence names the structure and the
//! field. See `docs/adr/0004-c-header-authorship-and-abi-verification.md` and
//! `docs/adr/0005-cpp-wrapper-shape-and-cmake-surface.md`.
//!
//! ```text
//! cargo build --locked --package mado-pilot-capi
//! cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
//! ```
//!
//! The build step is separate on purpose: this program needs the `cdylib` that
//! `cargo run` alone does not necessarily produce, and shelling back into cargo
//! from inside a cargo-launched process is a worse failure mode than a missing
//! artifact with an actionable message.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let label = label();
    let paths = Paths::discover()?;

    println!("host: {label}");
    println!("c compiler: {}", described(&paths.cc));
    println!("c++ compiler: {}", described(&paths.cxx));
    println!("cmake: {}", described(&paths.cmake));
    println!("rustc: {}", described(&rustc()));
    println!("library: {}", paths.library.display());

    check_layout(&paths)?;
    run_c_example(&paths, &label)?;
    check_cpp_ownership(&paths)?;
    run_cpp_example(&paths, &label)?;
    check_frozen_headers(&paths)?;
    check_cmake_consumer(&paths)?;

    println!("c-abi-check complete");

    Ok(())
}

/// Reads the `--label` argument, which names the host in the recorded output.
fn label() -> String {
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--label"
            && let Some(value) = arguments.next()
        {
            return value;
        }
    }

    "unlabelled host".to_owned()
}

/// Which compiler and which dialect a source is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    C,
    Cpp,
}

/// Everything this program has to find before it can do anything.
#[derive(Debug)]
struct Paths {
    /// The workspace root.
    root: PathBuf,
    /// The cargo profile directory the library was built into.
    artifacts: PathBuf,
    /// The built dynamic library.
    library: PathBuf,
    /// What to link against: the import library on Windows, the library itself
    /// elsewhere.
    link: PathBuf,
    /// Where this program writes the programs it builds.
    scratch: PathBuf,
    /// The C compiler.
    cc: OsString,
    /// The C++ compiler.
    cxx: OsString,
    /// The CMake executable.
    cmake: OsString,
}

impl Paths {
    fn discover() -> Result<Self, Box<dyn std::error::Error>> {
        // `<target>/<profile>/examples/c-abi-check`, so the profile directory
        // that holds the library is two levels up. Deriving it from the running
        // executable rather than from a guessed `target/debug` keeps a custom
        // `CARGO_TARGET_DIR` and a `--release` run working.
        let executable = env::current_exe()?;
        let artifacts = plain(
            executable
                .parent()
                .and_then(Path::parent)
                .ok_or("could not locate the cargo profile directory")?
                .to_path_buf(),
        );

        let root = plain(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .canonicalize()?,
        );

        let library_name = if cfg!(target_os = "windows") {
            "madopilot.dll"
        } else if cfg!(target_os = "macos") {
            "libmadopilot.dylib"
        } else {
            "libmadopilot.so"
        };

        let library = artifacts.join(library_name);
        if !library.exists() {
            return Err(format!(
                "{} does not exist.\nBuild it first:\n    cargo build --locked --package mado-pilot-capi",
                library.display()
            )
            .into());
        }

        let scratch = artifacts.join("c-abi-check");
        std::fs::create_dir_all(&scratch)?;

        Ok(Self {
            root,
            link: import_library(&artifacts, &library),
            artifacts,
            library,
            scratch,
            cc: compiler(Language::C),
            cxx: compiler(Language::Cpp),
            cmake: cmake()?,
        })
    }

    /// The directory holding `madopilot/madopilot.h` and `madopilot.hpp`.
    fn include(&self) -> PathBuf {
        self.root.join("crates/bindings/capi/include")
    }

    /// The directory holding `deterministic-scene.h`, which both examples and
    /// the C++ ownership probe include.
    fn shared_sources(&self) -> PathBuf {
        self.root.join("crates/bindings/capi/examples")
    }

    /// The include directory of one frozen header fixture.
    ///
    /// It holds a complete `madopilot/madopilot.h` of its own, so a program
    /// compiled with this in place of [`Paths::include`] cannot reach the
    /// working header even by accident.
    fn frozen_include(&self, version: &str) -> PathBuf {
        self.root
            .join("crates/bindings/capi/tests/abi-compat")
            .join(version)
    }

    fn program(&self, name: &str) -> PathBuf {
        self.scratch.join(if cfg!(target_os = "windows") {
            format!("{name}.exe")
        } else {
            name.to_owned()
        })
    }
}

/// Removes Windows' `\\?\` extended-length prefix from a canonicalized path.
///
/// `canonicalize` returns one on Windows, and MSVC's front end cannot open a
/// source or include path in that form: it reports C1083 as though the file did
/// not exist. Every path handed to a compiler goes through here.
fn plain(path: PathBuf) -> PathBuf {
    if let Some(text) = path.to_str()
        && let Some(stripped) = text.strip_prefix(r"\\?\")
        // A verbatim UNC path names a host as well as a path, and stripping the
        // prefix there would change what it means.
        && !stripped.starts_with("UNC\\")
    {
        return PathBuf::from(stripped);
    }

    path
}

/// Returns what a caller links against.
///
/// Everywhere but Windows that is the library itself. Cargo names a `cdylib`'s
/// Windows import library `<name>.dll.lib`; the undecorated `<name>.lib` is
/// accepted as well, so a future packaging change that renames it does not have
/// to touch this program.
fn import_library(artifacts: &Path, library: &Path) -> PathBuf {
    if !cfg!(target_os = "windows") {
        return library.to_path_buf();
    }

    let decorated = artifacts.join("madopilot.dll.lib");
    if decorated.exists() {
        return decorated;
    }

    artifacts.join("madopilot.lib")
}

/// Returns the compiler to use for a language.
///
/// `CC` and `CXX` first, so a host with more than one toolchain can say which.
/// Otherwise the release target's own compiler: MSVC on Windows, where one
/// driver builds both languages, and `cc`/`c++` on macOS, which are the Command
/// Line Tools clang.
fn compiler(language: Language) -> OsString {
    let configured = match language {
        Language::C => env::var_os("CC"),
        Language::Cpp => env::var_os("CXX"),
    };
    if let Some(configured) = configured {
        return configured;
    }
    if cfg!(target_os = "windows") {
        return OsString::from("cl");
    }

    match language {
        Language::C => OsString::from("cc"),
        Language::Cpp => OsString::from("c++"),
    }
}

/// Returns the CMake executable, and fails with an actionable message if there
/// is none.
///
/// `CMAKE` first. Then whatever is on `PATH`. On Windows, finally the copy
/// Visual Studio ships, which `vcvars64.bat` does not put on `PATH` — this
/// check already requires that environment, and `VSINSTALLDIR` is set inside
/// it, so the fallback costs nothing where it is needed.
fn cmake() -> Result<OsString, Box<dyn std::error::Error>> {
    if let Some(configured) = env::var_os("CMAKE") {
        if launches(&configured) {
            return Ok(configured);
        }
        return Err(format!(
            "`CMAKE` names `{}`, which could not be run.",
            configured.to_string_lossy()
        )
        .into());
    }

    let mut candidates = vec![OsString::from("cmake")];
    if let Some(root) = env::var_os("VSINSTALLDIR") {
        candidates.push(
            PathBuf::from(root)
                .join(r"Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe")
                .into_os_string(),
        );
    }

    for candidate in candidates {
        if launches(&candidate) {
            return Ok(candidate);
        }
    }

    let hint = if cfg!(target_os = "windows") {
        "\nVisual Studio ships one under \
         `%VSINSTALLDIR%\\Common7\\IDE\\CommonExtensions\\Microsoft\\CMake\\CMake\\bin`, \
         which is found automatically inside a Developer Command Prompt. Set `CMAKE` \
         to name one explicitly."
    } else {
        "\nSet `CMAKE` to name one explicitly."
    };

    Err(format!("no CMake 3.22 or later was found on `PATH`.{hint}").into())
}

fn launches(program: &OsString) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Returns `<name> (<version>)`, or just the name when it will not say.
///
/// The exact toolchain versions are a measurement condition: the evidence this
/// program produces is reproducible only if a reader knows which compilers
/// produced it. They were being discovered and thrown away — `launches` ran
/// `--version` and kept nothing but the exit status — which is why the tracked
/// evidence had to be annotated by hand.
///
/// Two conventions, because the compilers do not share one. MSVC prints its
/// banner on stderr when run with no arguments and treats `--version` as a
/// source file it cannot open; clang, gcc, CMake, and rustc answer `--version`
/// on stdout.
fn described(program: &OsString) -> String {
    let name = program.to_string_lossy().into_owned();
    let mut command = Command::new(program);
    let banner = is_msvc(program);
    if !banner {
        command.arg("--version");
    }

    let Ok(output) = command.output() else {
        return name;
    };
    let text = String::from_utf8_lossy(if banner {
        &output.stderr
    } else {
        &output.stdout
    });
    let Some(first) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return name;
    };

    format!("{name} ({first})")
}

/// Returns the `rustc` that built this program, so the report names the
/// compiler on both sides of the comparison.
fn rustc() -> OsString {
    env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"))
}

/// Returns the `ctest` beside a given `cmake`.
///
/// They are installed together, so the one next to the CMake actually in use is
/// the matching one even when another is on `PATH`.
fn ctest(cmake: &OsString) -> OsString {
    let path = PathBuf::from(cmake);
    let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return OsString::from("ctest");
    };

    parent
        .join(if cfg!(target_os = "windows") {
            "ctest.exe"
        } else {
            "ctest"
        })
        .into_os_string()
}

fn is_msvc(compiler: &OsString) -> bool {
    let name = compiler.to_string_lossy().to_lowercase();

    name == "cl" || name.ends_with("cl.exe") || name.ends_with("/cl") || name.ends_with("\\cl.exe")
}

/// Compiles `source` against the working header, linking the library when
/// asked.
fn compile(
    paths: &Paths,
    language: Language,
    name: &str,
    source: &Path,
    link: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    compile_with(paths, &paths.include(), language, name, source, link)
}

/// Compiles `source` against the headers in `include`.
///
/// The include directory is a parameter rather than a constant so that a frozen
/// header fixture can be compiled in place of the working one. Exactly one
/// header directory is passed, never both: a fixture that could fall through to
/// the working header would pass on the day it should fail.
fn compile_with(
    paths: &Paths,
    include: &Path,
    language: Language,
    name: &str,
    source: &Path,
    link: bool,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if !source.exists() {
        return Err(format!(
            "{} does not exist.\nThe C and C++ sources are tracked; check out the whole \
             `crates/bindings/capi/` directory.",
            source.display()
        )
        .into());
    }

    let compiler = match language {
        Language::C => &paths.cc,
        Language::Cpp => &paths.cxx,
    };
    let output = paths.program(name);
    // Named after the program rather than after the source: the C and C++
    // examples share a file name, and two object files called
    // `deterministic-slice.o` would be one object file.
    let object = paths.scratch.join(format!(
        "{name}.{}",
        if is_msvc(compiler) { "obj" } else { "o" }
    ));
    let mut command = Command::new(compiler);

    if is_msvc(compiler) {
        command.arg("/nologo");
        match language {
            Language::C => command.arg("/std:c11"),
            // `/EHsc` because the standard library this wrapper uses assumes
            // exceptions are enabled, even though the wrapper never throws one.
            Language::Cpp => command.arg("/std:c++17").arg("/EHsc"),
        };
        command
            .arg("/W3")
            .arg(format!("/I{}", include.display()))
            .arg(format!("/I{}", paths.shared_sources().display()))
            .arg(format!("/Fe:{}", output.display()))
            // Named explicitly rather than as a directory: an argument ending in
            // a separator is exactly what Windows command-line quoting mangles,
            // and `cl` would then write its object file into the current
            // directory instead.
            .arg(format!("/Fo:{}", object.display()))
            .arg(source);
        if link {
            command.arg(&paths.link);
        }
    } else {
        command.arg(match language {
            Language::C => "-std=c11",
            Language::Cpp => "-std=c++17",
        });
        command
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-I")
            .arg(include)
            .arg("-I")
            .arg(paths.shared_sources())
            .arg("-o")
            .arg(&output)
            .arg(source);
        if link {
            command
                .arg(&paths.library)
                // So the program finds the library at run time without the
                // caller having to set a search-path variable.
                .arg("-Wl,-rpath")
                .arg(format!("-Wl,{}", paths.artifacts.display()));
        }
    }

    let result = command.output().map_err(|error| {
        let hint = if cfg!(target_os = "windows") {
            "\nOn Windows, `cl` is only on `PATH` inside a Developer Command \
             Prompt or after `vcvars64.bat`. Set `CC` or `CXX` to use a different \
             compiler."
        } else {
            "\nSet `CC` or `CXX` to name a compiler explicitly."
        };
        format!(
            "could not run the compiler `{}`: {error}{hint}",
            compiler.to_string_lossy()
        )
    })?;
    report_output("compile", &result);
    if !result.status.success() {
        return Err(format!("compiling {} failed", source.display()).into());
    }

    Ok(output)
}

/// Runs a built program with the library reachable, and returns its output.
fn run(
    paths: &Paths,
    program: &Path,
    arguments: &[&str],
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut command = Command::new(program);
    command.args(arguments);
    reachable(paths, &mut command)?;

    Ok(command.output()?)
}

/// Puts the library's directory where the child process's loader will find it.
///
/// Windows has no rpath: the loader searches the executable's directory and then
/// `PATH`, so the library's directory is prepended for the child only.
fn reachable(paths: &Paths, command: &mut Command) -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(target_os = "windows") {
        let existing = env::var_os("PATH").unwrap_or_default();
        let mut search = vec![paths.artifacts.clone()];
        search.extend(env::split_paths(&existing));
        command.env("PATH", env::join_paths(search)?);
    }

    Ok(())
}

fn report_output(what: &str, output: &Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        println!("--- {what} stderr ---");
        print!("{stderr}");
    }
    if !output.status.success() && !stdout.trim().is_empty() {
        println!("--- {what} stdout ---");
        print!("{stdout}");
    }
}

/// Compiles and runs the layout probe, and diffs it against the Rust layout.
fn check_layout(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths
        .root
        .join("crates/bindings/capi/tests/c/madopilot-abi-layout.c");
    // The probe only includes the header, so it needs no library to link.
    let program = compile(paths, Language::C, "madopilot-abi-layout", &source, false)?;

    let output = run(paths, &program, &[])?;
    report_output("layout probe", &output);
    if !output.status.success() {
        return Err("the layout probe did not run".into());
    }

    let measured = String::from_utf8(output.stdout)?;
    let expected = madopilot::layout::report();

    print!("{measured}");

    let differences = diff(&expected, &measured);
    if !differences.is_empty() {
        for difference in &differences {
            println!("LAYOUT MISMATCH: {difference}");
        }
        return Err(format!(
            "the C header and the Rust definitions disagree in {} place(s)",
            differences.len()
        )
        .into());
    }

    println!(
        "layout: {} line(s) agree between rustc and the C compiler",
        expected.lines().count()
    );

    Ok(())
}

/// Compares two reports line by line, positionally as well as by content.
fn diff(expected: &str, measured: &str) -> Vec<String> {
    let expected: Vec<&str> = expected.lines().map(str::trim_end).collect();
    let measured: Vec<&str> = measured.lines().map(str::trim_end).collect();
    let mut differences = Vec::new();

    for (index, line) in expected.iter().enumerate() {
        match measured.get(index) {
            Some(found) if found == line => {}
            Some(found) => differences.push(format!("rust `{line}` but C `{found}`")),
            None => differences.push(format!("rust `{line}` but C reported nothing")),
        }
    }
    for line in measured.iter().skip(expected.len()) {
        differences.push(format!("C `{line}` but rust reported nothing"));
    }

    differences
}

/// The tracked deterministic asset package both examples load.
fn package(paths: &Paths) -> String {
    paths
        .root
        .join("fixtures/assets/phase1-slice")
        .to_string_lossy()
        .into_owned()
}

/// Compiles, links, and runs the C example against the built library.
fn run_c_example(paths: &Paths, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths
        .root
        .join("crates/bindings/capi/examples/c/deterministic-slice.c");
    let program = compile(paths, Language::C, "deterministic-slice-c", &source, true)?;

    let package = package(paths);
    let output = run(paths, &program, &["--package", &package, "--label", label])?;

    check_example("C", &output)
}

/// Compiles, links, and runs the C++ example, which answers the same questions.
fn run_cpp_example(paths: &Paths, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths
        .root
        .join("crates/bindings/capi/examples/cpp/deterministic-slice.cpp");
    let program = compile(
        paths,
        Language::Cpp,
        "deterministic-slice-cpp",
        &source,
        true,
    )?;

    let package = package(paths);
    let output = run(paths, &program, &["--package", &package, "--label", label])?;

    check_example("C++", &output)
}

/// Checks an example's outcome, and that it reached the end.
///
/// Each example checks its own expectations and exits non-zero on the first
/// surprise. These lines guard against one exiting zero without having got
/// there, and are the same for both languages because both must answer the same
/// question with the same numbers.
fn check_example(language: &str, output: &Output) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    print!("{stdout}");
    report_output(&format!("{language} example"), output);

    if !output.status.success() {
        return Err(format!("the {language} example reported a failure").into());
    }

    for required in [
        "deterministic slice complete",
        "absent template: 0 match(es)",
        "mapping still readable after close",
        "panel.patch at [20, 32) x [12, 22) score 1.000000",
        "panel.patch at [60, 72) x [40, 50) score 1.000000",
    ] {
        if !stdout.contains(required) {
            return Err(format!("the {language} example never printed `{required}`").into());
        }
    }

    Ok(())
}

/// Compiles and runs the C++ ownership probe.
///
/// Its static assertions are checked by the compile; its behaviour checks are
/// checked by the run.
fn check_cpp_ownership(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths
        .root
        .join("crates/bindings/capi/tests/cpp/madopilot-cpp-ownership.cpp");
    let program = compile(
        paths,
        Language::Cpp,
        "madopilot-cpp-ownership",
        &source,
        true,
    )?;

    let package = package(paths);
    let output = run(paths, &program, &["--package", &package])?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    print!("{stdout}");
    report_output("C++ ownership probe", &output);

    if !output.status.success() {
        return Err("the C++ ownership probe reported a failure".into());
    }
    if !stdout.contains("madopilot-cpp-ownership complete") {
        return Err("the C++ ownership probe never reached the end".into());
    }

    Ok(())
}

/// Every released header this library still promises to serve.
///
/// One entry per frozen ABI-major header. A later phase that adds entries adds
/// a fixture beside the existing ones rather than editing them, so the list
/// only ever grows and each entry keeps saying what one released header saw.
const FROZEN_HEADERS: &[&str] = &["v1"];

/// Compiles, links, negotiates, and runs each frozen header's fixture against
/// the library built now.
///
/// The fixture is compiled with its own include directory *instead of* the
/// working one, so it cannot reach the current header. That is the whole
/// mechanism: the day the working header gains an entry, this program still
/// compiles against the frozen declarations, and negotiation is what tells it
/// how much of the table it may use.
fn check_frozen_headers(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    for version in FROZEN_HEADERS {
        let include = paths.frozen_include(version);
        let source = include.join("old-prefix.c");
        let name = format!("madopilot-abi-compat-{version}");
        let program = compile_with(paths, &include, Language::C, &name, &source, true)?;

        let package = package(paths);
        let output = run(paths, &program, &["--package", &package])?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        print!("{stdout}");
        report_output(&name, &output);

        if !output.status.success() {
            return Err(format!("the frozen {version} header no longer works").into());
        }
        if !stdout.contains(&format!("{name} complete")) {
            return Err(format!("the frozen {version} fixture never reached the end").into());
        }
    }

    println!(
        "abi compatibility: {} frozen header(s) still compile, link, negotiate, and run",
        FROZEN_HEADERS.len()
    );

    Ok(())
}

/// Configures, builds, and runs the CMake consumer project.
///
/// It is a separate project with its own cache: it knows the two target names
/// and nothing else, so a `MadoPilot::Cpp` that failed to carry its include
/// directory or to bring `MadoPilot::C` with it would fail here.
fn check_cmake_consumer(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths.root.join("crates/bindings/capi/tests/cmake");
    let build = paths.scratch.join("cmake");
    let package = paths.root.join("crates/bindings/capi");

    // One configuration on both single-config and multi-config generators: the
    // former reads CMAKE_BUILD_TYPE, the latter ignores it and takes `--config`.
    // No generator is named, so each host uses the one it has — Ninja is not
    // guaranteed on either runner, and MSBuild and Unix Makefiles are.
    let configure = cmake_step(
        paths,
        &paths.cmake.clone(),
        &[
            OsString::from("-S"),
            source.clone().into_os_string(),
            OsString::from("-B"),
            build.clone().into_os_string(),
            OsString::from(format!("-DMADOPILOT_SOURCE_DIR={}", package.display())),
            OsString::from(format!(
                "-DMADOPILOT_ARTIFACT_DIR={}",
                paths.artifacts.display()
            )),
            OsString::from("-DCMAKE_BUILD_TYPE=Release"),
        ],
        "cmake configure",
    )?;
    if !configure.status.success() {
        return Err("configuring the CMake consumer project failed".into());
    }

    let built = cmake_step(
        paths,
        &paths.cmake.clone(),
        &[
            OsString::from("--build"),
            build.clone().into_os_string(),
            OsString::from("--config"),
            OsString::from("Release"),
        ],
        "cmake build",
    )?;
    if !built.status.success() {
        return Err("building the CMake consumer project failed".into());
    }

    let tested = cmake_step(
        paths,
        &ctest(&paths.cmake),
        &[
            OsString::from("--test-dir"),
            build.into_os_string(),
            OsString::from("--build-config"),
            OsString::from("Release"),
            OsString::from("--output-on-failure"),
        ],
        "ctest",
    )?;
    let stdout = String::from_utf8_lossy(&tested.stdout).into_owned();
    if !tested.status.success() {
        print!("{stdout}");
        return Err("the CMake consumer tests failed".into());
    }

    println!("cmake: the consumer project built and both consumers ran");

    Ok(())
}

fn cmake_step(
    paths: &Paths,
    program: &OsString,
    arguments: &[OsString],
    what: &str,
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut command = Command::new(program);
    command.args(arguments);
    reachable(paths, &mut command)?;

    let output = command
        .output()
        .map_err(|error| format!("could not run `{}`: {error}", program.to_string_lossy()))?;
    report_output(what, &output);

    Ok(output)
}
