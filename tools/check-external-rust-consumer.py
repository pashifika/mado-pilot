#!/usr/bin/env python3
"""Build a physically external Rust consumer and run only its model-free smoke path.

Requires Python 3.13+, installed Rust 1.97.1 (rustfmt/clippy), and the existing
setup-native.py prerequisites. --work-root must be a new directory outside the
checkout; no attempt, output, or caller data is ever removed. This proves the
path-source build route, not Git consumption, capture, input, or deployment.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parent / "native-release-profile"))
from process_runner import run_process  # noqa: E402


PACKAGE = "mado-pilot-input-workflow"
TOOLCHAIN = "1.97.1"
DEPLOYMENT = "26.5.2"
DEPENDENCY = b'mado-pilot = { path = "../../crates/mado-pilot" }'
OUTPUT_LIMIT = 16 * 1024 * 1024
BASE_KEYS = frozenset(name.upper() for name in (
    "HOME", "USERPROFILE", "HOMEDRIVE", "HOMEPATH", "SystemRoot", "WINDIR",
    "COMSPEC", "PATHEXT", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP", "TMPDIR",
    "LANG", "LC_ALL", "LC_CTYPE", "NUMBER_OF_PROCESSORS", "PROCESSOR_ARCHITECTURE",
    "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432",
))
ENVIRONMENT_OBSERVER = (
    "import json, os, sys; names = set(json.loads(sys.argv[1])); "
    "print(json.dumps(sorted(name.upper() for name in os.environ "
    "if name.upper() in names)))"
)


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")


def regular_files(root: Path) -> list[Path]:
    """Do not follow links or copy previous Cargo output into a new proof."""
    files = []
    for directory, directories, names in os.walk(root, followlinks=False):
        parent = Path(directory)
        for name in directories + names:
            path = parent / name
            if path.is_symlink() or path.is_junction():
                raise ValueError(f"source links are not allowed: {path}")
        directories[:] = sorted(name for name in directories if name not in ("target", ".git"))
        for name in sorted(names):
            path = parent / name
            if not path.is_file():
                raise ValueError(f"source must contain only regular files: {path}")
            files.append(path)
    return sorted(files)


def fresh_root(requested: Path, checkout: Path) -> Path:
    if any(ord(character) < 32 or ord(character) == 127 for character in str(requested)):
        raise ValueError("--work-root must not contain control characters")
    if requested.exists() or requested.is_symlink() or requested.is_junction():
        raise ValueError("--work-root already exists; select a new path (nothing was removed)")
    root = requested.parent.resolve(strict=True) / requested.name
    if root.is_relative_to(checkout):
        raise ValueError("--work-root must be physically outside the product checkout")
    for ancestor in root.parents:
        for name in ("config", "config.toml"):
            if (ancestor / ".cargo" / name).exists():
                raise ValueError(f"external Cargo cwd would inherit {ancestor / '.cargo' / name}")
    root.mkdir(mode=0o700)
    return root


def copy_consumer(source: Path, destination: Path, checkout: Path) -> dict:
    if source.is_symlink() or source.is_junction():
        raise ValueError("consumer source root must not be a link")
    files = regular_files(source)
    required = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "src/main.rs")
    if not all((source / name).is_file() for name in required):
        raise ValueError("complete consumer sources and its committed Cargo.lock are required")
    destination.mkdir()
    original_hashes = {}
    for path in files:
        relative = path.relative_to(source)
        copied = destination / relative
        copied.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, copied)
        original_hashes[relative.as_posix()] = digest(copied)
    manifest = destination / "Cargo.toml"
    original = manifest.read_bytes()
    if original.splitlines().count(DEPENDENCY) != 1:
        raise ValueError("consumer must contain exactly the documented mado-pilot path dependency line")
    absolute = json.dumps((checkout / "crates/mado-pilot").as_posix(), ensure_ascii=False)
    replacement = f"mado-pilot = {{ path = {absolute} }}".encode()
    manifest.write_bytes(re.sub(rb"(?m)^" + re.escape(DEPENDENCY) + rb"(?=\r?$)",
                                lambda _match: replacement, original, count=1))
    package = tomllib.loads(manifest.read_text(encoding="utf-8"))
    if package.get("package", {}).get("name") != PACKAGE or package["package"].get("edition") != "2024":
        raise ValueError("consumer package name and edition must match the documented contract")
    if package.get("workspace", {}).get("resolver") != "3":
        raise ValueError("consumer must own a resolver-3 workspace")
    toolchain = tomllib.loads((destination / "rust-toolchain.toml").read_text(encoding="utf-8"))
    if toolchain.get("toolchain", {}).get("channel") != TOOLCHAIN:
        raise ValueError(f"consumer must pin its own Rust {TOOLCHAIN}")
    config = tomllib.loads((destination / ".cargo/config.toml").read_text(encoding="utf-8"))
    if config != {"env": {"MACOSX_DEPLOYMENT_TARGET": {"value": DEPLOYMENT, "force": True}}}:
        raise ValueError("consumer Cargo config must contain only its explicit forced deployment floor")
    return {"original_sha256": original_hashes,
            "external_sha256": {path.relative_to(destination).as_posix(): digest(path)
                                for path in regular_files(destination)},
            "manifest_change": "only the documented facade dependency path becomes absolute"}


def baseline_environment(rustup: Path, work: Path) -> dict[str, str]:
    environment = {key: value for key, value in os.environ.items() if key.upper() in BASE_KEYS}
    if sys.platform == "win32":
        system = Path(os.environ["SystemRoot"])
        paths = [rustup.parent, system / "System32", system, system / "System32/Wbem"]
    else:
        paths = [rustup.parent, Path("/usr/bin"), Path("/bin"), Path("/usr/sbin"), Path("/sbin")]
    environment.update(
        PATH=os.pathsep.join(map(str, paths)),
        RUSTUP_HOME=str(Path(os.environ.get("RUSTUP_HOME", Path.home() / ".rustup")).resolve(strict=True)),
        RUSTUP_TOOLCHAIN=TOOLCHAIN,
        RUSTUP_AUTO_INSTALL="0",
        CARGO_HOME=str(work / "cargo-home"),
        CARGO_TARGET_DIR=str(work / "target"),
        CARGO_TERM_COLOR="never",
        CARGO_INCREMENTAL="0",
        TMPDIR=str(work / "tmp"),
        TEMP=str(work / "tmp"),
        TMP=str(work / "tmp"),
        PYTHONDONTWRITEBYTECODE="1",
    )
    return environment


def apply_native_exports(baseline: dict[str, str], env_file: Path, path_file: Path) -> tuple[dict, list[str]]:
    environment = dict(baseline)
    names = []
    for line in env_file.read_text(encoding="utf-8").splitlines():
        if not line:
            continue
        name, separator, value = line.partition("=")
        if not separator or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_-]*", name):
            raise ValueError("native setup emitted an invalid environment assignment")
        if name.upper() in {key.upper() for key in baseline} or name.upper() in names:
            raise ValueError(f"native setup may not override baseline field {name}")
        names.append(name.upper())
        environment[name] = value
    paths = [line for line in path_file.read_text(encoding="utf-8").splitlines() if line]
    environment["PATH"] = os.pathsep.join([*reversed(paths), baseline["PATH"]])
    return environment, sorted(names)


def assert_graph(metadata: dict, consumer: Path, checkout: Path, target: Path) -> dict:
    if Path(metadata["workspace_root"]).resolve() != consumer:
        raise ValueError("Cargo selected a workspace other than the external consumer")
    if Path(metadata["target_directory"]).resolve() != target:
        raise ValueError("Cargo selected a target directory other than the fresh dedicated root")
    packages = {package["id"]: package for package in metadata["packages"]}
    root = metadata["resolve"]["root"]
    if metadata["workspace_members"] != [root] or packages[root]["name"] != PACKAGE:
        raise ValueError("external Cargo workspace must contain only the consumer")
    facade = (checkout / "crates/mado-pilot/Cargo.toml").resolve()
    direct = packages[root]["dependencies"]
    product = [dependency for dependency in direct
               if dependency["name"].startswith("mado-pilot") or dependency.get("path")]
    if (len(product) != 1 or product[0]["name"] != "mado-pilot"
            or Path(product[0].get("path", "")).resolve() != facade.parent
            or product[0]["kind"] is not None):
        raise ValueError("the facade must be the consumer's only direct product/path dependency")
    rows = []
    for node in metadata["resolve"]["nodes"]:
        package = packages[node["id"]]
        name = package["name"]
        if name == "mado-pilot-testkit":
            raise ValueError("external consumer resolved the private testkit")
        if name.startswith("mado-pilot") and name != PACKAGE:
            manifest = Path(package["manifest_path"]).resolve()
            if package["source"] is not None or not manifest.is_relative_to(checkout / "crates"):
                raise ValueError(f"product dependency does not use the selected checkout: {name}")
            if any(re.search(r"fixture|qualification|benchmark|testkit", feature, re.IGNORECASE)
                   for feature in node["features"]):
                raise ValueError(f"private fixture/qualification/benchmark features resolved for {name}")
            rows.append({"name": name, "manifest": str(manifest), "manifest_sha256": digest(manifest),
                         "features": node["features"]})
    if not any(row["name"] == "mado-pilot" and Path(row["manifest"]) == facade for row in rows):
        raise ValueError("selected facade is absent from the resolved graph")
    return {"workspace_root": str(consumer), "target_directory": str(target), "product_packages": rows}


def deployment_version(output: str) -> str:
    versions = re.findall(r"^\s*minos\s+(\d+(?:\.\d+){1,2})\s*$", output, re.MULTILINE)
    if versions != [DEPLOYMENT]:
        raise ValueError(f"expected one Mach-O deployment minimum {DEPLOYMENT}, observed {versions}")
    return versions[0]


def prove(args: argparse.Namespace, checkout: Path, work: Path) -> int:
    evidence = work / "evidence"
    evidence.mkdir()
    consumer = work / "consumer"
    report = {"schema_version": 1, "status": "failed", "source_route": "path",
              "checkout": str(checkout), "work_root": str(work), "toolchain": TOOLCHAIN,
              "scope": "independent source build and model-free consumer decisions only",
              "not_exercised": ["Git dependency route", "models", "engine", "discovery", "permissions",
                                "capture", "input", "application postconditions", "minimum-host deployment"],
              "commands": []}

    def save() -> None:
        write_json(evidence / "result.json", report)

    def command(label: str, argv: list[str], environment: dict[str, str], *,
                seconds: int = 120, expected: int = 0) -> str:
        row = {"id": label, "argv": argv, "cwd": str(consumer), "expected_exit": expected,
               "timeout_seconds": seconds, "output_limit_bytes": OUTPUT_LIMIT, "status": "running"}
        report["commands"].append(row)
        save()
        print(f"[{label}] {json.dumps(argv, ensure_ascii=True)}", flush=True)
        result = run_process(argv, cwd=consumer, env=environment, timeout_seconds=seconds,
                             output_limit_bytes=OUTPUT_LIMIT, cleanup_seconds=15)
        for stream in ("stdout", "stderr"):
            (evidence / f"{label}.{stream}.log").write_text(result[stream], encoding="utf-8")
            if result[stream]:
                print(result[stream], end="" if result[stream].endswith("\n") else "\n", flush=True)
        row["process"] = {key: value for key, value in result.items() if key not in ("stdout", "stderr")}
        passed = (result["exit_code"] == expected and not result["timed_out"] and not result["output_limited"]
                  and result["cleanup_ok"] and result["launch_error"] is None)
        row["status"] = "passed" if passed else "failed"
        save()
        if not passed:
            raise ValueError(f"{label} failed; retained command/output: {evidence}")
        return result["stdout"]

    lock_hash = None
    try:
        save()
        if sys.version_info < (3, 13):
            raise ValueError("Python 3.13+ is required")
        report["consumer"] = copy_consumer(checkout / "examples/rust-input-workflow", consumer, checkout)
        lock_hash = digest(consumer / "Cargo.lock")
        shutil.copyfile(consumer / "Cargo.lock", evidence / "consumer.Cargo.lock")
        shutil.copyfile(checkout / "examples/rust-input-workflow/Cargo.toml", evidence / "original-Cargo.toml")
        rustup_path = shutil.which("rustup")
        if rustup_path is None:
            raise ValueError("rustup and the installed pinned toolchain are required")
        rustup = Path(rustup_path).absolute()
        baseline = baseline_environment(rustup, work)
        (work / "cargo-home").mkdir()
        (work / "target").mkdir()
        (work / "tmp").mkdir()
        git = shutil.which("git")
        if git is None:
            raise ValueError("git is required to record the source checkout identity")
        identity = command("source-identity", [git, "-C", str(checkout), "rev-parse", "HEAD", "HEAD^{tree}"], baseline)
        revision, tree = identity.splitlines()
        report["source"] = {"revision": revision, "tree": tree}
        command("source-status", [git, "-C", str(checkout), "status", "--porcelain=v1", "--untracked-files=normal"], baseline)
        command("product-source-diff", [git, "-C", str(checkout), "diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD", "--",
                                        "Cargo.toml", "Cargo.lock", "crates", "tools/setup-native.py",
                                        "tools/native-release-profile"], baseline)
        report["procedure_sha256"] = {str(path.relative_to(checkout)): digest(path) for path in (
            checkout / "tools/check-external-rust-consumer.py", checkout / "tools/setup-native.py",
            checkout / "tools/native-release-profile/process_runner.py",
            checkout / "tools/native-release-profile/_windows_process.py",
            checkout / "tools/native-release-profile/_process_group.py",
        )}
        compiler = Path(command("rustc-path", [str(rustup), "which", "--toolchain", TOOLCHAIN, "rustc"],
                                baseline).strip()).resolve(strict=True)
        # Cargo subprocesses also need the toolchain when rustup proxies are absent.
        baseline["PATH"] = os.pathsep.join((str(compiler.parent), baseline["PATH"]))
        rustc = command("rustc-version", [str(rustup), "run", TOOLCHAIN, "rustc", "-vV"], baseline)
        command("cargo-version", [str(rustup), "run", TOOLCHAIN, "cargo", "-V"], baseline)
        native = {("darwin", "arm64"): "aarch64-apple-darwin",
                  ("win32", "amd64"): "x86_64-pc-windows-msvc"}.get((sys.platform, platform.machine().lower()))
        if native is None or f"host: {native}" not in rustc.splitlines() or f"release: {TOOLCHAIN}" not in rustc.splitlines():
            raise ValueError("the selected compiler must be Rust 1.97.1 on a supported native release host")
        report["target"] = native
        env_file, path_file = evidence / "native.env", evidence / "native.path"
        setup_environment = dict(baseline)
        setup_environment["PATH"] = os.environ.get("PATH", os.defpath)
        for name in ("INCLUDE", "LIB", "LIBPATH", "VSCMD_ARG_TGT_ARCH"):
            if name in os.environ:
                setup_environment[name] = os.environ[name]
        setup = [sys.executable, "-B", str(checkout / "tools/setup-native.py"),
                 "--github-env", str(env_file), "--github-path", str(path_file)]
        for name in ("opencv_root", "libclang_path"):
            if value := getattr(args, name):
                setup += ["--" + name.replace("_", "-"), str(value.resolve(strict=True))]
        native_report = json.loads(command("native-setup", setup, setup_environment, seconds=240))
        write_json(evidence / "native-setup.json", native_report)
        environment, exported_names = apply_native_exports(baseline, env_file, path_file)
        observe = [sys.executable, "-I", "-c", ENVIRONMENT_OBSERVER, json.dumps(exported_names)]
        before = json.loads(command("baseline-environment", observe, baseline))
        after = json.loads(command("exported-environment", observe, environment))
        if before or after != exported_names:
            raise ValueError("native child environment must be reconstructed solely from setup export files")
        report["environment"] = {"baseline_fields": sorted(baseline), "native_fields_before": before,
                                 "native_fields_after": after, "path": environment["PATH"],
                                 "cargo_home": environment["CARGO_HOME"], "target_dir": environment["CARGO_TARGET_DIR"]}
        cargo = [str(rustup), "run", TOOLCHAIN, "cargo"]
        metadata = json.loads(command("cargo-metadata", [*cargo, "metadata", "--locked", "--format-version", "1",
                                                        "--filter-platform", native], environment, seconds=1800))
        write_json(evidence / "cargo-metadata.json", metadata)
        report["graph"] = assert_graph(metadata, consumer, checkout, work / "target")
        command("cargo-feature-tree", [*cargo, "tree", "--locked", "--package", PACKAGE,
                                        "--target", native, "--edges", "features"], environment)
        command("consumer-format", [*cargo, "fmt", "--package", PACKAGE, "--", "--check"], environment)
        scoped = ["--locked", "--package", PACKAGE, "--bin", PACKAGE, "--target", native]
        command("consumer-check", [*cargo, "check", *scoped], environment, seconds=1800)
        command("consumer-clippy", [*cargo, "clippy", *scoped, "--no-deps", "--", "-D", "warnings"],
                environment, seconds=1800)
        command("consumer-build", [*cargo, "build", *scoped], environment, seconds=1800)
        binary = work / "target" / native / "debug" / (PACKAGE + (".exe" if sys.platform == "win32" else ""))
        report["binary"] = {"path": str(binary), "sha256": digest(binary), "bytes": binary.stat().st_size}
        if sys.platform == "darwin":
            load_commands = command("deployment-metadata", ["/usr/bin/otool", "-l", str(binary)], environment)
            report["deployment_minimum"] = deployment_version(load_commands)
        command("consumer-help", [str(binary), "--help"], environment)
        command("consumer-smoke", [str(binary), "--smoke"], environment)
        report["status"] = "passed"
    except (OSError, ValueError, KeyError, TypeError) as error:
        report["error"] = str(error)
        print(f"external consumer proof failed: {error}", file=sys.stderr)
    except KeyboardInterrupt:
        report["error"] = "interrupted; no success is claimed"
    finally:
        if lock_hash is not None:
            final_hash = digest(consumer / "Cargo.lock") if (consumer / "Cargo.lock").is_file() else None
            report["lockfile"] = {"original_sha256": lock_hash, "final_sha256": final_hash,
                                  "unchanged": final_hash == lock_hash}
            if final_hash != lock_hash:
                report["status"] = "failed"
                report["error"] = "consumer lockfile changed during --locked proof"
        save()
    print(f"{report['status']}: evidence retained at {evidence}", flush=True)
    return 0 if report["status"] == "passed" else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work-root", required=True, type=Path, metavar="NEW-DIR",
                        help="new external directory under an existing parent; never reused or deleted")
    parser.add_argument("--opencv-root", type=Path, metavar="DIR", help="forward installed OpenCV root to native setup")
    parser.add_argument("--libclang-path", type=Path, metavar="DIR", help="forward installed libclang directory to native setup")
    args = parser.parse_args()
    checkout = Path(__file__).resolve().parents[1]
    try:
        work = fresh_root(args.work_root, checkout)
    except (OSError, ValueError) as error:
        print(f"external consumer proof refused: {error}", file=sys.stderr)
        return 1
    return prove(args, checkout, work)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    raise SystemExit(main())
