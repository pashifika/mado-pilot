//! Captures source and compiler provenance only for maintainer benchmarks.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "benchmark-instrumentation")]
    provenance::generate();
}

#[cfg(feature = "benchmark-instrumentation")]
mod provenance {
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const UNAVAILABLE: &str = "unavailable";
    const SOURCE_PATHS: &[&str] = &[
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        ".cargo",
        "crates",
        "tools/ocr-models",
        "fixtures/ocr",
    ];

    pub(super) fn generate() {
        let manifest =
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest dir"));
        let compiler = compiler(&manifest).unwrap_or_else(|| UNAVAILABLE.to_owned());
        let source = source(&manifest);
        let (revision, tree, dirty) = source.unwrap_or_else(|| {
            println!("cargo::warning=benchmark source provenance unavailable; evaluation refused");
            (UNAVAILABLE.to_owned(), UNAVAILABLE.to_owned(), true)
        });
        if compiler == UNAVAILABLE {
            println!(
                "cargo::warning=benchmark compiler provenance unavailable; evaluation refused"
            );
        }
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
        fs::write(
            out.join("model_evaluation_build.rs"),
            format!(
                "pub(super) const BUILD_SOURCE_REVISION: &str = {revision:?};\n\
                 pub(super) const BUILD_SOURCE_TREE: &str = {tree:?};\n\
                 pub(super) const BUILD_COMPILER: &str = {compiler:?};\n\
                 pub(super) const BUILD_SOURCE_DIRTY: bool = {dirty};\n"
            ),
        )
        .expect("write benchmark build provenance");
    }

    fn watch(path: &Path) {
        println!("cargo::rerun-if-changed={}", path.display());
    }

    fn compiler(manifest: &Path) -> Option<String> {
        for name in [
            "RUSTC",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTUP_TOOLCHAIN",
            "RUSTUP_HOME",
            "CARGO_HOME",
            "PATH",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
        ] {
            println!("cargo::rerun-if-env-changed={name}");
        }
        let rustc = env::var_os("RUSTC")?;
        let wrapper = |name| env::var_os(name).filter(|value| !value.is_empty());
        // Match Cargo's outer-wrapper, workspace-wrapper, compiler ordering.
        let commands: Vec<OsString> = [
            wrapper("RUSTC_WRAPPER"),
            wrapper("RUSTC_WORKSPACE_WRAPPER"),
            Some(rustc),
        ]
        .into_iter()
        .flatten()
        .collect();
        for command in &commands {
            let path = Path::new(command);
            if path.is_absolute() {
                watch(path);
            }
        }
        let output = Command::new(&commands[0])
            .args(&commands[1..])
            .arg("-Vv")
            .current_dir(manifest)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    fn git(directory: &Path, arguments: &[&str]) -> Option<Vec<u8>> {
        let output = Command::new("git")
            .arg("--no-optional-locks")
            .args(["-c", "core.fsmonitor=false"])
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .ok()?;
        output.status.success().then_some(output.stdout)
    }

    fn git_text(directory: &Path, arguments: &[&str]) -> Option<String> {
        let text = String::from_utf8(git(directory, arguments)?).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }

    fn object_id(directory: &Path, revision: &str) -> Option<String> {
        git_text(directory, &["rev-parse", "--verify", revision])
            .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    }

    fn source(manifest: &Path) -> Option<(String, String, bool)> {
        let root = PathBuf::from(git_text(manifest, &["rev-parse", "--show-toplevel"])?);
        // A packaged crate inside some other checkout is not that checkout's source.
        if root.join("crates/backend/onnx").canonicalize().ok()? != manifest.canonicalize().ok()? {
            return None;
        }
        for path in SOURCE_PATHS {
            watch(&root.join(path));
        }
        let tracked = git(&root, &["ls-files", "--cached", "-z"])?;
        for path in tracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            watch(&root.join(std::str::from_utf8(path).ok()?));
        }
        let git_dir = PathBuf::from(git_text(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-dir"],
        )?);
        let common_dir = PathBuf::from(git_text(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?);
        if root.join(".git").is_file() {
            watch(&root.join(".git"));
        }
        for path in ["HEAD", "index"] {
            watch(&git_dir.join(path));
        }
        for (directory, paths) in [
            (&git_dir, &["commondir", "config.worktree"][..]),
            (
                &common_dir,
                &["packed-refs", "config", "info", "refs", "reftable"][..],
            ),
        ] {
            for path in paths {
                let path = directory.join(path);
                if path.exists() {
                    watch(&path);
                }
            }
        }
        let head = fs::read_to_string(git_dir.join("HEAD")).ok()?;
        if let Some(reference) = head.trim().strip_prefix("ref: ") {
            // HEAD itself does not change when committing on the same branch.
            for directory in [&common_dir, &git_dir] {
                let path = directory.join(reference);
                if path.exists() {
                    watch(&path);
                }
            }
        }
        let revision = object_id(&root, "HEAD^{commit}")?;
        let tree = object_id(&root, &format!("{revision}^{{tree}}"))?;
        let dirty = git(
            &root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
        )
        .is_none_or(|status| !status.is_empty());
        // Do not join facts from two revisions if HEAD moved during collection.
        if object_id(&root, "HEAD^{commit}")? != revision {
            return None;
        }
        Some((revision, tree, dirty))
    }
}
