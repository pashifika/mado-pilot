//! Compiles the C header against this library and checks that they agree.
//!
//! The header is hand-written, so its agreement with the Rust `#[repr(C)]`
//! definitions is proved rather than assumed. This program:
//!
//! 1. compiles and runs `tests/c/madopilot-abi-layout.c`, which reports sizes,
//!    alignments, and field offsets as the C compiler produced them;
//! 2. compares that report line by line against the same values measured from
//!    the Rust definitions;
//! 3. compiles, links, and runs `examples/c/deterministic-slice.c` against the
//!    built library and checks its outcome.
//!
//! Two compilers, one comparison. A divergence names the structure and the
//! field. See `docs/adr/0004-c-header-authorship-and-abi-verification.md`.
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
    println!("compiler: {}", paths.compiler.to_string_lossy());
    println!("library: {}", paths.library.display());

    check_layout(&paths)?;
    run_example(&paths, &label)?;

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
    compiler: OsString,
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
            compiler: compiler(),
        })
    }

    fn include(&self) -> PathBuf {
        self.root.join("crates/bindings/capi/include")
    }

    /// The directory holding `deterministic-scene.h`, which the example
    /// includes.
    fn shared_sources(&self) -> PathBuf {
        self.root.join("crates/bindings/capi/examples")
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

/// Returns the C compiler to use.
///
/// `CC` first, so a host with more than one toolchain can say which. Otherwise
/// the release target's own compiler: MSVC on Windows, and whatever `cc` is on
/// macOS, which is the Command Line Tools clang.
fn compiler() -> OsString {
    if let Some(configured) = env::var_os("CC") {
        return configured;
    }
    if cfg!(target_os = "windows") {
        return OsString::from("cl");
    }

    OsString::from("cc")
}

fn is_msvc(compiler: &OsString) -> bool {
    let name = compiler.to_string_lossy().to_lowercase();

    name == "cl" || name.ends_with("cl.exe") || name.ends_with("/cl") || name.ends_with("\\cl.exe")
}

/// Compiles `source` into `output`, linking the library when asked.
fn compile(
    paths: &Paths,
    source: &Path,
    output: &Path,
    link: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !source.exists() {
        return Err(format!(
            "{} does not exist.\nThe C sources are tracked; check out the whole \
             `crates/bindings/capi/` directory.",
            source.display()
        )
        .into());
    }

    let object = paths.scratch.join(
        source
            .with_extension(if is_msvc(&paths.compiler) { "obj" } else { "o" })
            .file_name()
            .ok_or("the source has no file name")?,
    );
    let mut command = Command::new(&paths.compiler);

    if is_msvc(&paths.compiler) {
        command
            .arg("/nologo")
            .arg("/std:c11")
            .arg("/W3")
            .arg(format!("/I{}", paths.include().display()))
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
        command
            .arg("-std=c11")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-I")
            .arg(paths.include())
            .arg("-I")
            .arg(paths.shared_sources())
            .arg("-o")
            .arg(output)
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

    let output = command.output().map_err(|error| {
        let hint = if cfg!(target_os = "windows") {
            "\nOn Windows, `cl` is only on `PATH` inside a Developer Command \
             Prompt or after `vcvars64.bat`. Set `CC` to use a different compiler."
        } else {
            "\nSet `CC` to name a compiler explicitly."
        };
        format!(
            "could not run the C compiler `{}`: {error}{hint}",
            paths.compiler.to_string_lossy()
        )
    })?;
    report_output("compile", &output);
    if !output.status.success() {
        return Err(format!("compiling {} failed", source.display()).into());
    }

    Ok(())
}

/// Runs a built program with the library reachable, and returns its output.
fn run(
    paths: &Paths,
    program: &Path,
    arguments: &[&str],
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut command = Command::new(program);
    command.args(arguments);

    // Windows has no rpath: the loader searches the executable's directory and
    // then `PATH`, so the library's directory is prepended for the child only.
    if cfg!(target_os = "windows") {
        let existing = env::var_os("PATH").unwrap_or_default();
        let mut search = vec![paths.artifacts.clone()];
        search.extend(env::split_paths(&existing));
        command.env("PATH", env::join_paths(search)?);
    }

    Ok(command.output()?)
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
    let program = paths.program("madopilot-abi-layout");
    // The probe only includes the header, so it needs no library to link.
    compile(paths, &source, &program, false)?;

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

/// Compiles, links, and runs the C example against the built library.
fn run_example(paths: &Paths, label: &str) -> Result<(), Box<dyn std::error::Error>> {
    let source = paths
        .root
        .join("crates/bindings/capi/examples/c/deterministic-slice.c");
    let program = paths.program("deterministic-slice");
    compile(paths, &source, &program, true)?;

    let package = paths.root.join("fixtures/assets/phase1-slice");
    let package = package.to_string_lossy().into_owned();
    let output = run(paths, &program, &["--package", &package, "--label", label])?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    print!("{stdout}");
    report_output("example", &output);

    if !output.status.success() {
        return Err("the C example reported a failure".into());
    }

    // The example checks its own expectations, so this is a guard against it
    // exiting zero without having reached the end.
    for required in [
        "deterministic slice complete",
        "absent template: 0 match(es)",
        "mapping still readable after close",
    ] {
        if !stdout.contains(required) {
            return Err(format!("the C example never printed `{required}`").into());
        }
    }

    Ok(())
}
