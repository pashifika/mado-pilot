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
//! 4. compiles, links, and runs the current platform's native C example in its
//!    unattended, non-prompting mode;
//! 5. compiles, links, and runs `tests/cpp/madopilot-cpp-ownership.cpp`, whose
//!    static assertions prove the wrapper's ownership shape at compile time and
//!    whose checks prove its behaviour at run time;
//! 6. compiles, links, and runs `examples/cpp/deterministic-slice.cpp` and
//!    checks that it answers exactly what the C example answered;
//! 7. compiles, links, and runs `examples/cpp/native-input.cpp` in the same safe
//!    native mode through the C++ wrapper;
//! 8. compiles and runs the same layout probe a second time against each frozen
//!    header under `tests/abi-compat/`, and checks that every structure, field,
//!    and table entry that header declares is still where it said it was;
//! 9. compiles every frozen header fixture under `tests/abi-compat/` against
//!    its own header rather than the working one, links it to this library, and
//!    checks that it still negotiates and still gets the same answers;
//! 10. configures, builds, and runs the CMake consumer project in
//!     `tests/cmake/`, which reaches the library only through `MadoPilot::C` and
//!     `MadoPilot::Cpp`.
//!
//! Two compilers, one comparison. A divergence names the structure and the
//! field. See `docs/adr/0004-c-header-authorship-and-abi-verification.md` and
//! `docs/adr/0005-cpp-wrapper-shape-and-cmake-surface.md`.
//!
//! ```text
//! cargo build --locked --package mado-pilot-capi
//! cargo run --locked --package mado-pilot-capi --example c-abi-check -- --label "<host>"
//! # Windows fixture-backed mode additionally requires:
//! cargo build --locked --package mado-pilot-platform-windows \
//!   --bin mado-pilot-windows-input-fixture \
//!   --bin mado-pilot-windows-window-message-fixture
//! cargo run --locked --package mado-pilot-capi --example c-abi-check -- \
//!   --label "<host>" --windows-native-fixture
//! ```
//!
//! The build step is separate on purpose: this program needs the `cdylib` that
//! `cargo run` alone does not necessarily produce, and shelling back into cargo
//! from inside a cargo-launched process is a worse failure mode than a missing
//! artifact with an actionable message.

#[cfg(all(windows, feature = "qualification-unsupported-api"))]
use mado_pilot::{NativeEngineRequest, OperationContext, Status};

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[cfg(all(windows, feature = "qualification-unsupported-api"))]
const QUALIFY_MISSING_DIRECT3D_DEVICE: &str =
    "MADO_PILOT_WINDOWS_QUALIFY_MISSING_CREATE_DIRECT3D11_DEVICE";
#[cfg(all(windows, feature = "qualification-unsupported-api"))]
const UNSUPPORTED_RUST_CHILD: &str = "--windows-unsupported-rust-child";
#[cfg(all(windows, feature = "qualification-unsupported-api"))]
const SUPPORTED_RUST_CHILD: &str = "--windows-supported-rust-child";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(windows, feature = "qualification-unsupported-api"))]
    if let Some(mode) = env::args()
        .find(|argument| argument == UNSUPPORTED_RUST_CHILD || argument == SUPPORTED_RUST_CHILD)
    {
        return run_windows_rust_qualification_child(mode == UNSUPPORTED_RUST_CHILD);
    }
    let unsupported_qualification = windows_unsupported_qualification_requested();
    #[cfg(not(all(windows, feature = "qualification-unsupported-api")))]
    if unsupported_qualification {
        return Err(
            "`--windows-unsupported-qualification` requires a Windows qualification build".into(),
        );
    }
    #[cfg(all(windows, feature = "qualification-unsupported-api"))]
    if unsupported_qualification {
        check_windows_rust_qualification_children()?;
    }
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
    let run_windows_native_fixture = windows_native_fixture_requested();
    let native_fixtures = if run_windows_native_fixture {
        Some([
            WindowsNativeFixture::spawn(&paths, WindowsFixtureKind::Ordinary)?,
            WindowsNativeFixture::spawn(&paths, WindowsFixtureKind::Acknowledged)?,
        ])
    } else {
        None
    };
    match &native_fixtures {
        Some([ordinary, acknowledged]) => run_native_c_example(
            &paths,
            &[
                (
                    WindowsFixtureKind::Ordinary.contract_argument(),
                    ordinary.title(),
                ),
                (
                    WindowsFixtureKind::Acknowledged.contract_argument(),
                    acknowledged.title(),
                ),
            ],
        )?,
        None => run_native_c_example(&paths, &[])?,
    }
    drop(native_fixtures);
    check_cpp_ownership(&paths)?;
    run_cpp_example(&paths, &label)?;
    #[cfg(feature = "private-fixture")]
    run_ocr_fixture_examples(&paths)?;
    run_default_ocr_examples(&paths)?;
    let native_fixtures = if run_windows_native_fixture {
        Some([
            WindowsNativeFixture::spawn(&paths, WindowsFixtureKind::Ordinary)?,
            WindowsNativeFixture::spawn(&paths, WindowsFixtureKind::Acknowledged)?,
        ])
    } else {
        None
    };
    match &native_fixtures {
        Some([ordinary, acknowledged]) => run_native_cpp_example(
            &paths,
            &[
                (
                    WindowsFixtureKind::Ordinary.contract_argument(),
                    ordinary.title(),
                ),
                (
                    WindowsFixtureKind::Acknowledged.contract_argument(),
                    acknowledged.title(),
                ),
            ],
        )?,
        None => run_native_cpp_example(&paths, &[])?,
    }
    check_frozen_layout(&paths)?;
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

/// Whether this run must launch and exercise the dedicated Windows fixture.
fn windows_native_fixture_requested() -> bool {
    env::args().any(|argument| argument == "--windows-native-fixture")
}

fn windows_unsupported_qualification_requested() -> bool {
    env::args().any(|argument| argument == "--windows-unsupported-qualification")
}

#[cfg(all(windows, feature = "qualification-unsupported-api"))]
fn run_windows_rust_qualification_child(
    expect_unsupported: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let engine = mado_pilot::windows_engine(NativeEngineRequest::new())?;
    let result = engine.discover(&OperationContext::new());
    if expect_unsupported {
        let error = result.expect_err("the controlled missing export refuses discovery");
        if error.status() != Status::Unsupported {
            return Err(format!(
                "the controlled missing export returned {:?}, not Unsupported",
                error.status()
            )
            .into());
        }
        println!("rust native unsupported discovery complete");
    } else {
        let _targets = result?;
        println!("rust native supported discovery complete");
    }
    Ok(())
}

#[cfg(all(windows, feature = "qualification-unsupported-api"))]
fn check_windows_rust_qualification_children() -> Result<(), Box<dyn std::error::Error>> {
    let current = env::current_exe()?;
    for (argument, unsupported, expected) in [
        (
            UNSUPPORTED_RUST_CHILD,
            true,
            "rust native unsupported discovery complete",
        ),
        (
            SUPPORTED_RUST_CHILD,
            false,
            "rust native supported discovery complete",
        ),
    ] {
        let mut command = Command::new(&current);
        command.arg(argument);
        if unsupported {
            command.env(QUALIFY_MISSING_DIRECT3D_DEVICE, "1");
        }
        let output = command.output()?;
        report_output("Windows Rust availability child", &output);
        let stdout = String::from_utf8(output.stdout)?;
        print!("{stdout}");
        if !output.status.success() || !stdout.contains(expected) {
            return Err(format!("Windows Rust availability child `{argument}` failed").into());
        }
    }
    Ok(())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsFixtureKind {
    Ordinary,
    Acknowledged,
}

impl WindowsFixtureKind {
    #[cfg(windows)]
    const fn program(self) -> &'static str {
        match self {
            Self::Ordinary => "mado-pilot-windows-window-message-fixture",
            Self::Acknowledged => "mado-pilot-windows-input-fixture",
        }
    }

    const fn contract_argument(self) -> &'static str {
        match self {
            Self::Ordinary => "--ordinary",
            Self::Acknowledged => "--acknowledged",
        }
    }
}

/// One repository-owned Windows fixture kept alive for a native language example.
struct WindowsNativeFixture {
    #[cfg(windows)]
    child: std::process::Child,
    title: String,
}

impl WindowsNativeFixture {
    fn spawn(paths: &Paths, kind: WindowsFixtureKind) -> Result<Self, Box<dyn std::error::Error>> {
        #[cfg(not(windows))]
        {
            let _ = (paths, kind);
            Err("`--windows-native-fixture` requires a Windows host".into())
        }

        #[cfg(windows)]
        {
            use std::process::Stdio;

            let program =
                paths
                    .artifacts
                    .join(format!("{}{}", kind.program(), env::consts::EXE_SUFFIX));
            if !program.is_file() {
                return Err(format!(
                    "{} does not exist.\nBuild it first:\n    cargo build --locked \
                     --package mado-pilot-platform-windows --bin {}",
                    program.display(),
                    kind.program()
                )
                .into());
            }

            let mut command = Command::new(&program);
            if kind == WindowsFixtureKind::Acknowledged {
                command.arg("--animate-on-input");
            }
            let child = command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;
            let mut fixture = Self {
                child,
                title: String::new(),
            };
            let process_id = fixture.child.id();
            let output = fixture
                .child
                .stdout
                .take()
                .ok_or("the Windows fixture did not expose its readiness output")?;
            let ready = read_windows_fixture_ready(output)?;
            let title = ready
                .strip_prefix("fixture-ready ")
                .and_then(|line| line.split_once(" title="))
                .and_then(|(_, rest)| rest.split_once(" capacity="))
                .map(|(title, _)| title)
                .ok_or("the Windows fixture returned malformed readiness output")?;
            if !title.ends_with(&format!("[{process_id}]")) {
                return Err("the Windows fixture title did not identify its process".into());
            }
            fixture.title = title.to_owned();
            println!(
                "windows {:?} fixture: ready for exact-title native checks",
                kind
            );
            Ok(fixture)
        }
    }

    fn title(&self) -> &str {
        &self.title
    }
}

#[cfg(windows)]
fn read_windows_fixture_ready(
    output: std::process::ChildStdout,
) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    const READY_BUDGET: Duration = Duration::from_secs(5);

    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut ready = String::new();
        let result = BufReader::new(output).read_line(&mut ready).map(|_| ready);
        let _sent = sender.send(result);
    });
    match receiver.recv_timeout(READY_BUDGET) {
        Ok(result) => Ok(result?),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err("the Windows fixture timed out before readiness".into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("the Windows fixture readiness reader stopped".into())
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsNativeFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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

/// The C program that reports the layout its compiler produced.
///
/// It is compiled twice: once against the working header, and once per frozen
/// header. Both runs are compared against the same Rust measurements.
fn layout_probe(paths: &Paths) -> PathBuf {
    paths
        .root
        .join("crates/bindings/capi/tests/c/madopilot-abi-layout.c")
}

/// Compiles and runs the layout probe, and diffs it against the Rust layout.
fn check_layout(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let source = layout_probe(paths);
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

#[cfg(feature = "private-fixture")]
fn ocr_package(paths: &Paths) -> String {
    paths
        .root
        .join("fixtures/assets/ocr-public-surface")
        .to_string_lossy()
        .into_owned()
}

/// Runs the C and C++ OCR examples against the explicit private fixture backend.
#[cfg(feature = "private-fixture")]
fn run_ocr_fixture_examples(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let c_source = paths
        .root
        .join("crates/bindings/capi/examples/c/ocr-fixture.c");
    let c = compile(paths, Language::C, "ocr-fixture-c", &c_source, true)?;
    let cpp_source = paths
        .root
        .join("crates/bindings/capi/examples/cpp/ocr-fixture.cpp");
    let cpp = compile(paths, Language::Cpp, "ocr-fixture-cpp", &cpp_source, true)?;
    let package = ocr_package(paths);
    let c_output = run(paths, &c, &["--package", &package])?;
    let cpp_output = run(paths, &cpp, &["--package", &package])?;
    report_output("OCR fixture C", &c_output);
    report_output("OCR fixture C++", &cpp_output);
    if !c_output.status.success() || !cpp_output.status.success() {
        return Err("an OCR fixture example failed".into());
    }
    let c_stdout = String::from_utf8(c_output.stdout)?.replace("\r\n", "\n");
    let cpp_stdout = String::from_utf8(cpp_output.stdout)?.replace("\r\n", "\n");
    print!("{c_stdout}");
    print!("{cpp_stdout}");
    let expected = "ocr: sequence=0 text=魔導士 A-7 confidence=0.91000\n";
    if c_stdout != expected || cpp_stdout != expected {
        return Err("C/C++ OCR fixture observations diverged".into());
    }
    println!("OCR fixture examples agree");
    Ok(())
}

/// Compiles both production-language default OCR examples and runs them when
/// the caller supplied the reviewed native prerequisites.
fn run_default_ocr_examples(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let c_source = paths
        .root
        .join("crates/bindings/capi/examples/c/ocr-default.c");
    let c = compile(paths, Language::C, "ocr-default-c", &c_source, true)?;
    let cpp_source = paths
        .root
        .join("crates/bindings/capi/examples/cpp/ocr-default.cpp");
    let cpp = compile(paths, Language::Cpp, "ocr-default-cpp", &cpp_source, true)?;

    let (Some(model_root), Some(runtime)) = (
        env::var_os("MADO_PILOT_G004_MODEL_ROOT"),
        env::var_os("MADO_PILOT_ONNX_RUNTIME"),
    ) else {
        println!("default OCR C/C++ examples compiled; native run skipped without explicit paths");
        return Ok(());
    };
    let model_root = PathBuf::from(model_root).canonicalize()?;
    let runtime = PathBuf::from(runtime).canonicalize()?;
    let model_root = model_root.to_string_lossy().into_owned();
    let runtime = runtime.to_string_lossy().into_owned();
    let arguments = [
        "--model-root",
        model_root.as_str(),
        "--runtime",
        runtime.as_str(),
    ];
    let c_output = run(paths, &c, &arguments)?;
    let cpp_output = run(paths, &cpp, &arguments)?;
    report_output("default OCR C", &c_output);
    report_output("default OCR C++", &cpp_output);
    if !c_output.status.success() || !cpp_output.status.success() {
        return Err("an integrated default OCR example failed".into());
    }
    let c_stdout = String::from_utf8(c_output.stdout)?.replace("\r\n", "\n");
    let cpp_stdout = String::from_utf8(cpp_output.stdout)?.replace("\r\n", "\n");
    print!("{c_stdout}");
    print!("{cpp_stdout}");
    let expected = format!(
        "default-ocr: backend={} model={} full=0 region=0\n",
        mado_pilot::DEFAULT_OCR_BACKEND_ID,
        mado_pilot::ACCEPTED_G004_MODEL_ID,
    );
    if c_stdout != expected || cpp_stdout != expected {
        return Err("C/C++ default OCR observations diverged".into());
    }
    println!("default OCR C/C++ examples agree");
    Ok(())
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

/// Compiles, links, and runs the current release target's native C probe.
///
/// With no targets, `--check` stops before discovery and sends no input. Each
/// target is one exact repository fixture title and its required contract.
fn run_native_c_example(
    paths: &Paths,
    targets: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    let name = if cfg!(target_os = "windows") {
        "windows-native-input"
    } else if cfg!(target_os = "macos") {
        "macos-native-input"
    } else {
        return Err("native C examples require a release-target host".into());
    };
    let source = paths
        .root
        .join("crates/bindings/capi/examples/c")
        .join(format!("{name}.c"));
    let program = compile(paths, Language::C, name, &source, true)?;
    #[cfg(all(windows, feature = "qualification-unsupported-api"))]
    if windows_unsupported_qualification_requested() {
        check_unsupported_native_program(paths, &program, name, "C")?;
    }
    if targets.is_empty() {
        check_native_program(paths, &program, name, "C", None)?;
    } else {
        for &target in targets {
            check_native_program(paths, &program, name, "C", Some(target))?;
        }
    }
    Ok(())
}

fn check_native_program(
    paths: &Paths,
    program: &Path,
    name: &str,
    language: &str,
    target: Option<(&str, &str)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let output = match target {
        Some((contract, title)) => run(paths, program, &[contract, title])?,
        None => run(paths, program, &["--check"])?,
    };
    let stdout = String::from_utf8(output.stdout.clone())?;
    print!("{stdout}");
    report_output(&format!("native {language} example"), &output);

    let mode = target.map_or("non-prompting check", |_| "fixture-backed flow");
    if !output.status.success() {
        return Err(format!("the {name} {mode} reported a failure").into());
    }
    let expected = if target.is_some() {
        format!("{name} complete")
    } else {
        format!("{name} complete (non-prompting check)")
    };
    if !stdout.contains(&expected) {
        return Err(format!("the {name} {mode} never reached the end").into());
    }
    if let Some((contract, _)) = target {
        let contract = contract.trim_start_matches("--");
        if !stdout.contains(&format!("contract: {contract}")) {
            return Err(format!(
                "the {name} {mode} did not verify the requested {contract} contract"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(all(windows, feature = "qualification-unsupported-api"))]
fn check_unsupported_native_program(
    paths: &Paths,
    program: &Path,
    name: &str,
    language: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::new(program);
    command.arg("--unsupported-check");
    command.env(QUALIFY_MISSING_DIRECT3D_DEVICE, "1");
    reachable(paths, &mut command)?;
    let output = command.output()?;
    report_output(&format!("unsupported native {language} example"), &output);
    let stdout = String::from_utf8(output.stdout)?;
    print!("{stdout}");
    if !output.status.success() || !stdout.contains(&format!("{name} complete (unsupported check)"))
    {
        return Err(format!("the {name} unsupported native {language} check failed").into());
    }
    Ok(())
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

/// Compiles, links, and runs the native flow through the C++ RAII wrapper.
fn run_native_cpp_example(
    paths: &Paths,
    targets: &[(&str, &str)],
) -> Result<(), Box<dyn std::error::Error>> {
    let name = if cfg!(target_os = "windows") {
        "windows-native-input-cpp"
    } else if cfg!(target_os = "macos") {
        "macos-native-input-cpp"
    } else {
        return Err("the native C++ example requires a release-target host".into());
    };
    let source = paths
        .root
        .join("crates/bindings/capi/examples/cpp/native-input.cpp");
    let program = compile(paths, Language::Cpp, name, &source, true)?;
    #[cfg(all(windows, feature = "qualification-unsupported-api"))]
    if windows_unsupported_qualification_requested() {
        check_unsupported_native_program(paths, &program, name, "C++")?;
    }
    if targets.is_empty() {
        check_native_program(paths, &program, name, "C++", None)?;
    } else {
        for &target in targets {
            check_native_program(paths, &program, name, "C++", Some(target))?;
        }
    }
    Ok(())
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

/// Released header profiles whose frozen declarations remain ABI obligations.
///
/// ABI 1.0 and 1.2 are released frozen profiles older than the working ABI 1.3 header.
/// The unreleased ABI 1.1 draft has no compatibility fixture.
const FROZEN_HEADERS: &[&str] = &["v1", "v1_2"];

/// Runs the layout probe against each frozen header, and checks that what that
/// header declares is still true of the library built now.
///
/// [`check_layout`] proves the *working* header and the Rust definitions agree
/// with each other, which is weaker than it looks: a same-width field swap made
/// in both at once — `madopilot_result_info_t.backend_id` and
/// `backend_version`, or `madopilot_frame_stamp_t.epoch` and `sequence` — moves
/// no offset and leaves that comparison green while silently breaking every
/// caller built against the released header. The released header is the side a
/// v1 caller actually compiled against, so it is the side that has to be
/// measured against the library, and nothing did that before: the frozen
/// fixture `old-prefix.c` reads four values, and the probe saw only the working
/// include directory.
///
/// The comparison is containment rather than the positional diff
/// [`check_layout`] uses, because a later minor appends. Fields and unversioned
/// type extents remain exact. A type whose released prefix begins with
/// `struct_size` may grow, but its alignment may not change and its current size
/// may not be smaller than the released one. A field that moved, was renamed, or
/// was removed still fails here, and so does a table entry that changed position.
fn reported_type_layout(line: &str) -> Option<(&str, usize, usize)> {
    let rest = line.strip_prefix("type ")?;
    let (name, rest) = rest.split_once(" size=")?;
    let (size, align) = rest.split_once(" align=")?;
    Some((name, size.parse().ok()?, align.parse().ok()?))
}

fn check_frozen_layout(paths: &Paths) -> Result<(), Box<dyn std::error::Error>> {
    let source = layout_probe(paths);
    let report = madopilot::layout::report();
    let declared: HashSet<&str> = report.lines().map(str::trim_end).collect();

    for version in FROZEN_HEADERS {
        let include = paths.frozen_include(version);
        let name = format!("madopilot-abi-layout-{version}");
        // The probe only includes the header, so it needs no library to link.
        let program = compile_with(paths, &include, Language::C, &name, &source, false)?;

        let output = run(paths, &program, &[])?;
        report_output(&name, &output);
        if !output.status.success() {
            return Err(format!("the frozen {version} layout probe did not run").into());
        }

        let measured = String::from_utf8(output.stdout)?;
        let lines: Vec<&str> = measured
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect();
        let versioned: HashSet<&str> = lines
            .iter()
            .filter_map(|line| {
                line.strip_prefix("field ")?
                    .strip_suffix(".struct_size offset=0")
            })
            .collect();
        let mismatches: Vec<String> = lines
            .iter()
            .filter_map(|line| {
                if declared.contains(line) {
                    return None;
                }
                let Some((name, released_size, released_align)) = reported_type_layout(line)
                else {
                    return Some(format!(
                        "the {version} header declares `{line}`, which this library no longer reports"
                    ));
                };
                if !versioned.contains(name) {
                    return Some(format!(
                        "the {version} header declares unversioned `{line}`, which this library no longer reports"
                    ));
                }

                let current = report
                    .lines()
                    .filter_map(reported_type_layout)
                    .find(|(current_name, _, _)| *current_name == name);
                match current {
                    Some((_, current_size, current_align))
                        if current_size >= released_size && current_align == released_align =>
                    {
                        None
                    }
                    Some((_, current_size, current_align)) => Some(format!(
                        "the {version} header declares `{name}` size={released_size} \
                         align={released_align}, but this library reports size={current_size} \
                         align={current_align}"
                    )),
                    None => Some(format!(
                        "the {version} header declares type `{name}`, which this library no longer reports"
                    )),
                }
            })
            .collect();

        if !mismatches.is_empty() {
            for mismatch in &mismatches {
                println!("FROZEN LAYOUT MISMATCH: {mismatch}");
            }
            return Err(format!(
                "the frozen {version} header and this library disagree in {} place(s)",
                mismatches.len()
            )
            .into());
        }

        println!(
            "frozen layout: all {} line(s) the {version} header declares still hold against \
             this library",
            lines.len()
        );
    }

    Ok(())
}

/// Compiles, links, and runs every released historical header fixture against
/// the library built now.
///
/// The fixture is compiled with its own include directory *instead of* the
/// working one, so it cannot reach the current header. Every released fixture
/// must still negotiate and run.
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
            return Err(format!("the released {version} header fixture failed").into());
        }
        if !stdout.contains(&format!("{name} complete")) {
            return Err(format!("the released {version} fixture never reached the end").into());
        }
    }

    println!(
        "abi history: {} frozen header fixture(s) passed their expected negotiation behavior",
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
    // CMake paths use forward slashes on every host. Passing `Path::display()`
    // directly leaves Windows separators in an untyped `-D` value; older
    // supported CMake releases then parse sequences such as `\W` as invalid
    // escapes when the value is expanded into a source path.
    let package = package.to_string_lossy().replace('\\', "/");
    let artifacts = paths.artifacts.to_string_lossy().replace('\\', "/");

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
            OsString::from(format!("-DMADOPILOT_SOURCE_DIR={package}")),
            OsString::from(format!("-DMADOPILOT_ARTIFACT_DIR={artifacts}")),
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

    println!("cmake: the consumer project built and all consumers ran");

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
